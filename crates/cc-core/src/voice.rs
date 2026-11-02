//! Push-to-talk from the dashboard: the browser records, Whisper on the
//! server writes it down, and the words are handled like a Pocket command.
//! "Claude, ..." or a trigger word goes where it names; "add a todo ..." or
//! "remind me ..." does that; anything else goes to the agent on screen.

use std::sync::Arc;

use base64::Engine;
use serde::Serialize;

use crate::{commands, db, pocket, whisper, Core};

/// Longest recording accepted, as uploaded (about 10 minutes of speech).
const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Debug)]
pub struct Heard {
    /// What Whisper wrote down.
    pub heard: String,
    /// "agent", "todo", "reminder" or "project".
    pub kind: String,
    /// One line on what happened, for anything but an agent's answer.
    pub result: String,
    /// The agent run, when the words went to an agent.
    pub run: Option<db::AgentRun>,
}

/// Who answers when the words don't name anyone: the agent or AI on screen.
#[derive(Default, Debug)]
pub struct Fallback {
    pub agent_id: Option<i64>,
    pub provider: Option<String>,
}

pub async fn handle_audio(core: &Arc<Core>, audio_b64: &str, mime: &str, fallback: Fallback) -> Result<Heard, String> {
    let audio = base64::engine::general_purpose::STANDARD.decode(audio_b64.trim()).map_err(|_| "The recording didn't arrive in one piece.")?;
    if audio.is_empty() {
        return Err("The recording was empty. Check that the browser can use your microphone.".into());
    }
    if audio.len() > MAX_AUDIO_BYTES {
        return Err("That recording is too long. Keep it to a few minutes.".into());
    }
    let text = whisper::transcribe(audio, mime).await?;
    if text.trim().is_empty() {
        return Err("I didn't catch anything. Try again a little closer to the mic.".into());
    }
    handle_text(core, &text, fallback).await
}

pub async fn handle_text(core: &Arc<Core>, text: &str, fallback: Fallback) -> Result<Heard, String> {
    let (wake, agents, triggers) = {
        let conn = core.db.0.lock().unwrap();
        let agents: Vec<(i64, String)> = db::list_agents(&conn).map_err(|e| e.to_string())?.into_iter().map(|a| (a.id, a.name)).collect();
        (pocket::wake(&conn), agents, pocket::triggers(&conn))
    };
    // Saying the wake phrase first is fine, just not needed.
    let said = pocket::extract(text, &wake).into_iter().next().unwrap_or_else(|| text.trim().to_string());
    let p = pocket::parse_with(&said, &agents, &triggers, chrono::Local::now());
    let heard = text.trim().to_string();

    let run = match p.kind.as_str() {
        "agent" => match p.target.split_once(':') {
            Some(("agent", id)) => commands::run_agent(core, id.parse().map_err(|_| "That agent no longer exists.")?, p.title.clone(), "voice").await?,
            Some(("provider", id)) => commands::ask_provider(core, id, p.title.clone(), "voice").await?,
            _ => return Err("Say who should answer, such as \"Claude, ...\".".into()),
        },
        // A todo the rules only guessed at is really a question for the agent on screen.
        "todo" if !p.sure => match (fallback.agent_id, fallback.provider.as_deref()) {
            (Some(id), _) => commands::run_agent(core, id, said.clone(), "voice").await?,
            (None, Some(provider)) => commands::ask_provider(core, provider, said.clone(), "voice").await?,
            // Nobody on screen: Local AI can answer, or add the todo itself.
            (None, None) => commands::ask_provider(core, "local", said.clone(), "voice").await?,
        },
        _ => {
            let kind = p.kind.clone();
            let c = pocket::run_heard(core, "mic", &said, p).await?;
            if c.status != "done" {
                return Err(c.result);
            }
            return Ok(Heard { heard, kind, result: c.result, run: None });
        }
    };
    Ok(Heard { heard, kind: "agent".into(), result: String::new(), run: Some(run) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, Secrets};
    use crate::SignIn;

    fn core() -> Arc<Core> {
        let dir = std::env::temp_dir().join(format!("cc-voice-{}", rand::random::<u64>()));
        Core::open(&dir, Secrets::new(MemoryStore::default()), SignIn::DeviceCode).unwrap()
    }

    #[tokio::test]
    async fn spoken_tasks_run_and_questions_go_to_the_agent_on_screen() {
        let core = core();
        let h = handle_text(&core, "Add a todo to renew the domain.", Fallback::default()).await.unwrap();
        assert_eq!((h.kind.as_str(), h.result.as_str()), ("todo", "Todo added: Renew the domain"));
        // The wake phrase may lead.
        let h = handle_text(&core, "Command Center, remind me at 3 PM to call Dana", Fallback::default()).await.unwrap();
        assert_eq!(h.kind, "reminder");
        // A question with nobody named and nobody on screen goes to Local AI
        // (not running here, so the run says so).
        let e = handle_text(&core, "What's the tallest mountain in Colorado?", Fallback::default()).await;
        assert!(e.as_ref().is_err_and(|e| !e.contains("Say who should answer")) || e.as_ref().is_ok_and(|h| h.run.is_some()), "{e:?}");
        // Named AIs are asked; without a key the run says what's missing.
        let e = handle_text(&core, "Grok, what's new?", Fallback::default()).await.unwrap_err();
        assert!(e.contains("Grok key"), "{e}");
        let fb = Fallback { agent_id: None, provider: Some("chatgpt".into()) };
        let e = handle_text(&core, "What's the tallest mountain in Colorado?", fb).await.unwrap_err();
        assert!(e.contains("ChatGPT key"), "{e}");
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }

    #[tokio::test]
    async fn rejects_empty_or_broken_audio() {
        let core = core();
        assert!(handle_audio(&core, "", "audio/webm", Fallback::default()).await.unwrap_err().contains("empty"));
        assert!(handle_audio(&core, "not base64!", "audio/webm", Fallback::default()).await.unwrap_err().contains("one piece"));
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }
}
