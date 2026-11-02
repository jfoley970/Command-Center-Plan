//! Local models through Ollama, running next to the server (or on this
//! computer). No key: Ollama is only reachable from inside the server, so the
//! address is the only setting.

use std::time::Duration;

use serde_json::{json, Value};

use crate::claude::{messages, Completion, History};

/// Where Ollama answers. On the server Compose sets `CC_OLLAMA_URL` to the
/// `ollama` service; without it, Ollama on this computer.
pub fn url() -> String {
    std::env::var("CC_OLLAMA_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434".into())
}

/// How many tokens the model sees at once (prompt plus answer). Ollama's own
/// default is small, and anything past it is cut from the front, so a long
/// chat would silently lose its start. 8k fits qwen3:8b beside Whisper on an
/// 8 GB card (with the q8 KV cache set in Compose). Every call sends the same
/// value so Ollama never reloads the model to change it.
pub fn num_ctx() -> u64 {
    std::env::var("CC_OLLAMA_NUM_CTX").ok().and_then(|v| v.trim().parse().ok()).filter(|n| *n >= 2048).unwrap_or(8192)
}

/// The models pulled into Ollama, as (name, label), smallest first.
pub async fn models() -> Result<Vec<(String, String)>, String> {
    let resp = reqwest::Client::new()
        .get(format!("{}/api/tags", url()))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| format!("Can't reach Ollama at {}.", url()))?;
    if !resp.status().is_success() {
        return Err(format!("Ollama at {} answered {}.", url(), resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| format!("Unexpected answer from Ollama: {e}"))?;
    Ok(parse_tags(&v))
}

fn parse_tags(v: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(u64, String, String)> = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            let size = m["size"].as_u64().unwrap_or(0);
            let params = m["details"]["parameter_size"].as_str().unwrap_or_default();
            let label = match (params.is_empty(), size) {
                (true, 0) => name.clone(),
                (true, s) => format!("{name} ({:.1} GB)", s as f64 / 1e9),
                (false, s) => format!("{name} ({params}, {:.1} GB)", s as f64 / 1e9),
            };
            Some((size, name, label))
        })
        .collect();
    out.sort_by_key(|m| m.0);
    out.into_iter().map(|(_, n, l)| (n, l)).collect()
}

fn chat_body(model: &str, system: &str, history: &History, user: &str) -> Value {
    let mut all = vec![json!({ "role": "system", "content": system })];
    all.extend(messages(history, user));
    json!({ "model": model, "messages": all, "stream": false, "options": { "num_ctx": num_ctx() } })
}

/// Drops `<think>…</think>` reasoning that some models leave in the answer, so
/// it isn't shown or sent back as chat history.
fn strip_thinking(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(start) = out.find("<think>") {
        match out[start..].find("</think>") {
            Some(end) => out.replace_range(start..start + end + "</think>".len(), ""),
            None => out.truncate(start),
        }
    }
    out.trim().to_string()
}

fn parse_chat(v: &Value) -> Completion {
    Completion {
        text: strip_thinking(v["message"]["content"].as_str().unwrap_or_default()),
        stop_reason: if v["done_reason"] == "length" { "max_tokens".into() } else { "end_turn".into() },
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["prompt_eval_count"].as_i64().unwrap_or(0),
        output_tokens: v["eval_count"].as_i64().unwrap_or(0),
    }
}

/// One answer from a local model. An empty model means the first one pulled.
pub async fn complete(model: &str, system: &str, history: &History, user: &str) -> Result<Completion, String> {
    let model = if model.trim().is_empty() {
        models().await?.into_iter().next().map(|m| m.0).ok_or("No local models yet. Pull one into Ollama first.")?
    } else {
        model.to_string()
    };
    // A model loading into memory, or running on CPU, can take minutes.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/chat", url()))
        .timeout(Duration::from_secs(900))
        .json(&chat_body(&model, system, history, user))
        .send()
        .await
        .map_err(|e| format!("Could not reach Ollama at {}: {e}", url()))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| format!("Unexpected answer from Ollama: {e}"))?;
    if !status.is_success() {
        let msg = v["error"].as_str().unwrap_or("unknown error");
        return Err(match status.as_u16() {
            404 => format!("{model} isn't pulled into Ollama. Pull it, or pick another model for this agent."),
            code => format!("Ollama error {code}: {msg}"),
        });
    }
    Ok(parse_chat(&v))
}

/// One step of a chat that can call tools: Ollama's answer message (which may
/// hold `tool_calls` instead of text) plus its token counts. Thinking is off, so
/// each step of a tool loop stays quick on a small GPU, unless `think` asks for
/// it (a retry after the model skipped a tool it needed).
pub async fn chat_step(model: &str, messages: &[Value], tools: &[Value], think: bool) -> Result<(Value, Completion), String> {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "think": think,
        "options": { "num_ctx": num_ctx() },
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let resp = reqwest::Client::new()
        .post(format!("{}/api/chat", url()))
        .timeout(Duration::from_secs(900))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Could not reach Ollama at {}: {e}", url()))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| format!("Unexpected answer from Ollama: {e}"))?;
    if !status.is_success() {
        let msg = v["error"].as_str().unwrap_or("unknown error");
        return Err(match status.as_u16() {
            404 => format!("{model} isn't pulled into Ollama. Pull it, or pick another model for this agent."),
            code => format!("Ollama error {code}: {msg}"),
        });
    }
    let completion = parse_chat(&v);
    Ok((v["message"].clone(), completion))
}

/// The tool calls in an answer message, as (name, arguments). Arguments that
/// arrive as a JSON string are parsed.
pub fn tool_calls(message: &Value) -> Vec<(String, Value)> {
    message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let f = &c["function"];
            let name = f["name"].as_str()?.to_string();
            let args = match &f["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                other => other.clone(),
            };
            Some((name, if args.is_object() { args } else { json!({}) }))
        })
        .collect()
}

/// Resolves an empty model name to the first model pulled.
pub async fn pick_model(model: &str) -> Result<String, String> {
    if model.trim().is_empty() {
        models().await?.into_iter().next().map(|m| m.0).ok_or_else(|| "No local models yet. Pull one into Ollama first.".to_string())
    } else {
        Ok(model.to_string())
    }
}

/// A short answer shaped by a JSON schema (Ollama's structured outputs), for
/// background jobs such as reading voice commands. Gives up after `timeout`.
pub async fn complete_json(model: &str, system: &str, user: &str, schema: Value, timeout: Duration) -> Result<Value, String> {
    structured(model, system, user, schema, timeout, false).await
}

/// Like `complete_json`, but asks the model to skip its thinking step, which
/// is most of the time a small job takes. Models that can't think are asked
/// again without the flag.
pub async fn complete_json_quick(model: &str, system: &str, user: &str, schema: Value, timeout: Duration) -> Result<Value, String> {
    match structured(model, system, user, schema.clone(), timeout, true).await {
        Err(e) if e.contains("think") => structured(model, system, user, schema, timeout, false).await,
        r => r,
    }
}

async fn structured(model: &str, system: &str, user: &str, schema: Value, timeout: Duration, no_think: bool) -> Result<Value, String> {
    let model = if model.trim().is_empty() {
        models().await?.into_iter().next().map(|m| m.0).ok_or("No local models yet. Pull one into Ollama first.")?
    } else {
        model.to_string()
    };
    let mut body = chat_body(&model, system, &[], user);
    body["format"] = schema;
    body["options"]["temperature"] = json!(0);
    if no_think {
        body["think"] = json!(false);
    }
    let resp = reqwest::Client::new()
        .post(format!("{}/api/chat", url()))
        .timeout(timeout)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Could not reach Ollama at {}: {e}", url()))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| format!("Unexpected answer from Ollama: {e}"))?;
    if !status.is_success() {
        return Err(format!("Ollama error {}: {}", status.as_u16(), v["error"].as_str().unwrap_or("unknown error")));
    }
    let text = parse_chat(&v).text;
    serde_json::from_str(&text).map_err(|e| format!("{model} didn't answer in the expected shape: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_sorted_smallest_first_with_labels() {
        let v = json!({ "models": [
            { "name": "qwen3:14b", "size": 9_300_000_000u64, "details": { "parameter_size": "14.8B" } },
            { "name": "llama3.2:3b", "size": 2_000_000_000u64, "details": { "parameter_size": "3.2B" } },
            { "name": "bare" },
        ]});
        let m = parse_tags(&v);
        assert_eq!(m[0], ("bare".into(), "bare".into()));
        assert_eq!(m[1].1, "llama3.2:3b (3.2B, 2.0 GB)");
        assert_eq!(m[2].0, "qwen3:14b");
    }

    #[test]
    fn chat_reply_parsed() {
        let v = json!({
            "model": "qwen3:8b",
            "message": { "role": "assistant", "content": " Hello. " },
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 3,
        });
        let c = parse_chat(&v);
        assert_eq!((c.text.as_str(), c.stop_reason.as_str(), c.input_tokens, c.output_tokens), ("Hello.", "end_turn", 12, 3));
        assert_eq!(parse_chat(&json!({ "done_reason": "length" })).stop_reason, "max_tokens");
        let body = chat_body("m", "sys", &[], "hi");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][1]["content"], "hi");
        assert_eq!(body["options"]["num_ctx"], 8192);
    }

    #[test]
    fn tool_calls_are_read() {
        let m = json!({ "role": "assistant", "content": "", "tool_calls": [
            { "function": { "name": "add_todo", "arguments": { "title": "Call Dell" } } },
            { "function": { "name": "list_todos", "arguments": "{\"include_done\": true}" } },
            { "function": { "name": "overview", "arguments": null } },
            { "function": {} },
        ]});
        let calls = tool_calls(&m);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], ("add_todo".into(), json!({ "title": "Call Dell" })));
        assert_eq!(calls[1].1["include_done"], true);
        assert_eq!(calls[2].1, json!({}));
        assert!(tool_calls(&json!({ "content": "hi" })).is_empty());
    }

    #[test]
    fn thinking_is_left_out() {
        assert_eq!(strip_thinking("<think>hmm\nok</think>\n\nHi there."), "Hi there.");
        assert_eq!(strip_thinking("A <think>x</think>B<think>y</think> C"), "A B C");
        assert_eq!(strip_thinking("Cut off <think>never closed"), "Cut off");
        assert_eq!(strip_thinking("  plain "), "plain");
    }
}
