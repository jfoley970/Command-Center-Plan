//! Minimal Claude Messages API client. Rust has no official Anthropic SDK,
//! so this calls POST /v1/messages over HTTPS directly.

use serde::{Deserialize, Serialize};
use serde_json::json;

pub const DEFAULT_MODEL: &str = "claude-opus-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Lets the API retry a declined request on a suitable fallback model.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Models offered in the agent editor. The first is the default.
pub const MODELS: &[(&str, &str)] = &[
    ("claude-opus-5", "Claude Opus 5 (best quality)"),
    ("claude-sonnet-5", "Claude Sonnet 5 (faster, cheaper)"),
    ("claude-haiku-4-5", "Claude Haiku 4.5 (fastest, cheapest)"),
];

#[derive(Serialize, Clone, Debug)]
pub struct Completion {
    pub text: String,
    pub stop_reason: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    content: Vec<serde_json::Value>,
    stop_reason: Option<String>,
    stop_details: Option<serde_json::Value>,
    usage: Usage,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: i64,
    output_tokens: i64,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    message: String,
}

fn request_body(model: &str, system: &str, user: &str) -> serde_json::Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    // Server-side fallbacks are supported on the Opus 5 / Fable tiers only.
    if model.starts_with("claude-opus-5") || model.starts_with("claude-fable") {
        body["fallbacks"] = json!("default");
    }
    body
}

pub async fn complete(api_key: &str, model: &str, system: &str, user: &str) -> Result<Completion, String> {
    let body = request_body(model, system, user);
    let mut req = reqwest::Client::new()
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(600));
    if body.get("fallbacks").is_some() {
        req = req.header("anthropic-beta", FALLBACK_BETA);
    }

    let resp = req.json(&body).send().await.map_err(|e| format!("Could not reach the Claude API: {e}"))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        let msg = serde_json::from_slice::<ApiErrorBody>(&bytes)
            .map(|b| b.error.message)
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
        return Err(match status.as_u16() {
            401 => "The Claude API key was rejected. Check it in Settings.".into(),
            429 => format!("Rate limited by the Claude API, try again shortly. ({msg})"),
            code => format!("Claude API error {code}: {msg}"),
        });
    }

    let parsed: ApiResponse = serde_json::from_slice(&bytes).map_err(|e| format!("Unexpected API response: {e}"))?;
    let stop_reason = parsed.stop_reason.unwrap_or_default();

    let text = if stop_reason == "refusal" {
        let why = parsed
            .stop_details
            .as_ref()
            .and_then(|d| d.get("explanation"))
            .and_then(|e| e.as_str())
            .unwrap_or("no explanation given");
        format!("Claude declined this request: {why}")
    } else {
        parsed
            .content
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    Ok(Completion {
        text,
        stop_reason,
        model: parsed.model,
        input_tokens: parsed.usage.input_tokens,
        output_tokens: parsed.usage.output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_requests_carry_fallbacks() {
        let b = request_body("claude-opus-5", "sys", "hi");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["messages"][0]["content"], "hi");
        assert!(b.get("thinking").is_none());
    }

    #[test]
    fn other_models_do_not() {
        assert!(request_body("claude-sonnet-5", "s", "u").get("fallbacks").is_none());
        assert!(request_body("claude-haiku-4-5", "s", "u").get("fallbacks").is_none());
    }
}
