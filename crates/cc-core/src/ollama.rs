//! Local models through Ollama, running next to the server (or on this
//! computer). No key: Ollama is only reachable from inside the server, so the
//! address is the only setting.

use std::time::Duration;

use serde_json::{json, Value};

use crate::claude::Completion;

/// Where Ollama answers. On the server Compose sets `CC_OLLAMA_URL` to the
/// `ollama` service; without it, Ollama on this computer.
pub fn url() -> String {
    std::env::var("CC_OLLAMA_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434".into())
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

fn chat_body(model: &str, system: &str, user: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "stream": false,
    })
}

fn parse_chat(v: &Value) -> Completion {
    Completion {
        text: v["message"]["content"].as_str().unwrap_or_default().trim().to_string(),
        stop_reason: if v["done_reason"] == "length" { "max_tokens".into() } else { "end_turn".into() },
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["prompt_eval_count"].as_i64().unwrap_or(0),
        output_tokens: v["eval_count"].as_i64().unwrap_or(0),
    }
}

/// One answer from a local model. An empty model means the first one pulled.
pub async fn complete(model: &str, system: &str, user: &str) -> Result<Completion, String> {
    let model = if model.trim().is_empty() {
        models().await?.into_iter().next().map(|m| m.0).ok_or("No local models yet. Pull one into Ollama first.")?
    } else {
        model.to_string()
    };
    // A model loading into memory, or running on CPU, can take minutes.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/chat", url()))
        .timeout(Duration::from_secs(900))
        .json(&chat_body(&model, system, user))
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
        let body = chat_body("m", "sys", "hi");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][1]["content"], "hi");
    }
}
