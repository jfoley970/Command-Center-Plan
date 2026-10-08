//! OpenAI-style APIs: ChatGPT's features through OpenAI's Responses API, and
//! Grok through xAI's OpenAI-compatible Chat Completions API.

use serde_json::{json, Value};

use crate::claude::Completion;

pub const OPENAI_URL: &str = "https://api.openai.com/v1/responses";
pub const XAI_URL: &str = "https://api.x.ai/v1/chat/completions";

/// The ChatGPT tools an agent can use. Web search and running code are on for
/// every ChatGPT agent, like they are in the ChatGPT app.
fn responses_body(model: &str, system: &str, user: &str) -> Value {
    json!({
        "model": model,
        "instructions": system,
        "input": user,
        "tools": [
            { "type": "web_search" },
            { "type": "code_interpreter", "container": { "type": "auto" } },
        ],
    })
}

/// Pulls the text, source links and refusals out of a Responses API reply.
fn parse_responses(v: &Value) -> Completion {
    let mut text = Vec::new();
    let mut sources = Vec::<String>::new();
    let mut refused = None;
    for item in v["output"].as_array().into_iter().flatten() {
        if item["type"] != "message" {
            continue;
        }
        for part in item["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("output_text") => {
                    text.push(part["text"].as_str().unwrap_or_default().to_string());
                    for a in part["annotations"].as_array().into_iter().flatten() {
                        if a["type"] == "url_citation" {
                            if let Some(url) = a["url"].as_str() {
                                if !sources.iter().any(|s| s == url) {
                                    sources.push(url.to_string());
                                }
                            }
                        }
                    }
                }
                Some("refusal") => refused = part["refusal"].as_str().map(str::to_string),
                _ => {}
            }
        }
    }
    let mut out = text.join("\n\n");
    if !sources.is_empty() {
        out.push_str("\n\nSources:\n");
        out.push_str(&sources.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"));
    }
    let stop_reason = if let Some(why) = refused {
        out = format!("ChatGPT declined this request: {why}");
        "refusal".to_string()
    } else if v["status"] == "incomplete" {
        "max_tokens".to_string()
    } else {
        "end_turn".to_string()
    };
    Completion {
        text: out,
        stop_reason,
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
        output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
    }
}

fn chat_body(model: &str, system: &str, user: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    })
}

fn parse_chat(v: &Value) -> Completion {
    let choice = &v["choices"][0];
    let refusal = choice["message"]["refusal"].as_str();
    let (text, stop_reason) = match refusal {
        Some(why) => (format!("Grok declined this request: {why}"), "refusal".to_string()),
        None => (
            choice["message"]["content"].as_str().unwrap_or_default().to_string(),
            match choice["finish_reason"].as_str() {
                Some("length") => "max_tokens".to_string(),
                _ => "end_turn".to_string(),
            },
        ),
    };
    Completion {
        text,
        stop_reason,
        model: v["model"].as_str().unwrap_or_default().to_string(),
        input_tokens: v["usage"]["prompt_tokens"].as_i64().unwrap_or(0),
        output_tokens: v["usage"]["completion_tokens"].as_i64().unwrap_or(0),
    }
}

async fn post(url: &str, service: &str, api_key: &str, body: &Value) -> Result<Value, String> {
    let resp = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_secs(600))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Could not reach {service}: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("no details").to_string();
        return Err(match status.as_u16() {
            401 => format!("The {service} key was rejected. Check it in Settings > Connections."),
            429 => format!("{service} says you're over a rate or spend limit. ({msg})"),
            code => format!("{service} error {code}: {msg}"),
        });
    }
    Ok(v)
}

pub async fn chatgpt(api_key: &str, model: &str, system: &str, user: &str) -> Result<Completion, String> {
    let v = post(OPENAI_URL, "the OpenAI API", api_key, &responses_body(model, system, user)).await?;
    Ok(parse_responses(&v))
}

pub async fn grok(api_key: &str, model: &str, system: &str, user: &str) -> Result<Completion, String> {
    let v = post(XAI_URL, "the xAI API", api_key, &chat_body(model, system, user)).await?;
    Ok(parse_chat(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatgpt_requests_turn_on_search_and_code() {
        let b = responses_body("gpt-5", "be brief", "hi");
        assert_eq!(b["instructions"], "be brief");
        assert_eq!(b["input"], "hi");
        let tools: Vec<&str> = b["tools"].as_array().unwrap().iter().map(|t| t["type"].as_str().unwrap()).collect();
        assert_eq!(tools, ["web_search", "code_interpreter"]);
    }

    #[test]
    fn responses_text_and_sources_are_collected() {
        let v = json!({
            "model": "gpt-5-2025",
            "status": "completed",
            "output": [
                { "type": "web_search_call", "status": "completed" },
                { "type": "message", "content": [
                    { "type": "output_text", "text": "It's sunny.", "annotations": [
                        { "type": "url_citation", "url": "https://weather.example/a" },
                        { "type": "url_citation", "url": "https://weather.example/a" }
                    ]}
                ]}
            ],
            "usage": { "input_tokens": 12, "output_tokens": 5 }
        });
        let c = parse_responses(&v);
        assert_eq!(c.text, "It's sunny.\n\nSources:\n- https://weather.example/a");
        assert_eq!(c.stop_reason, "end_turn");
        assert_eq!((c.input_tokens, c.output_tokens), (12, 5));
    }

    #[test]
    fn responses_refusal_is_reported() {
        let v = json!({ "output": [{ "type": "message", "content": [{ "type": "refusal", "refusal": "No." }] }] });
        let c = parse_responses(&v);
        assert_eq!(c.stop_reason, "refusal");
        assert!(c.text.contains("No."));
    }

    #[test]
    fn chat_completions_parse() {
        let v = json!({
            "model": "grok-4",
            "choices": [{ "message": { "role": "assistant", "content": "Hello" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 1 }
        });
        let c = parse_chat(&v);
        assert_eq!(c.text, "Hello");
        assert_eq!(c.model, "grok-4");
        assert_eq!(c.output_tokens, 1);
        assert_eq!(chat_body("grok-4", "s", "u")["messages"][1]["content"], "u");
    }
}
