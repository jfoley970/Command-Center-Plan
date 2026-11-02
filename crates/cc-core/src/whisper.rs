//! Speech to text with Whisper, running next to the server like Ollama
//! (openai-whisper-asr-webservice in deploy/docker-compose.yml). No key: only
//! cc-server reaches it, so the recording never leaves the server.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// Where Whisper answers. On the server Compose sets `CC_WHISPER_URL` to the
/// `whisper` service; without it, Whisper on this computer.
pub fn url() -> String {
    std::env::var("CC_WHISPER_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:9000".into())
}

#[derive(Serialize)]
pub struct Status {
    pub connected: bool,
    pub note: String,
}

/// Whether Whisper is up. Its API page answers once the model has loaded.
pub async fn status() -> Status {
    let resp = reqwest::Client::new().get(format!("{}/docs", url())).timeout(Duration::from_secs(3)).send().await;
    match resp {
        Ok(r) if r.status().is_success() => Status { connected: true, note: "Whisper is ready".into() },
        Ok(r) => Status { connected: false, note: format!("Whisper at {} answered {}.", url(), r.status()) },
        Err(_) => Status { connected: false, note: format!("Can't reach Whisper at {}.", url()) },
    }
}

/// The words in a recording. `mime` is what the browser recorded (webm/opus
/// in Chrome and Edge, mp4 in Safari); Whisper converts it with ffmpeg.
pub async fn transcribe(audio: Vec<u8>, mime: &str) -> Result<String, String> {
    let ext = if mime.contains("mp4") || mime.contains("m4a") {
        "m4a"
    } else if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("wav") {
        "wav"
    } else {
        "webm"
    };
    let part = reqwest::multipart::Part::bytes(audio)
        .file_name(format!("voice.{ext}"))
        .mime_str(if mime.is_empty() { "application/octet-stream" } else { mime })
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new().part("audio_file", part);
    let resp = reqwest::Client::new()
        .post(format!("{}/asr", url()))
        .query(&[("output", "json"), ("encode", "true"), ("task", "transcribe"), ("language", "en"), ("vad_filter", "true")])
        .multipart(form)
        // The first request after a restart waits for the model to load.
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("Could not reach Whisper at {}: {e}", url()))?;
    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Whisper couldn't read the recording ({code}): {}", body.chars().take(200).collect::<String>()));
    }
    let v: Value = resp.json().await.map_err(|e| format!("Unexpected answer from Whisper: {e}"))?;
    Ok(text_of(&v))
}

fn text_of(v: &Value) -> String {
    v["text"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_text() {
        let v = serde_json::json!({ "text": "  Claude, what's\n the weather? ", "segments": [], "language": "en" });
        assert_eq!(text_of(&v), "Claude, what's the weather?");
        assert_eq!(text_of(&serde_json::json!({})), "");
    }

    #[tokio::test]
    async fn says_when_whisper_is_down() {
        std::env::set_var("CC_WHISPER_URL", "http://127.0.0.1:9");
        let s = status().await;
        assert!(!s.connected && s.note.contains("Can't reach Whisper"));
    }
}
