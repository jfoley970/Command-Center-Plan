//! The AI services agents can run on. Each agent belongs to one provider; the
//! provider decides which key it needs, which models it offers and how a run
//! is carried out. Adding a provider means one entry here and one branch in
//! `commands::run_agent`.

use serde::Serialize;

use crate::secrets::{self, Secrets};

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// Secret store entry holding its key, if it can be used yet.
    pub key: Option<&'static str>,
    /// Models offered in the agent editor. The first is the default.
    pub models: &'static [(&'static str, &'static str)],
    /// Works on a repository and finishes later, rather than answering in one call.
    pub background: bool,
    /// Shown when the provider can't be used from the app yet.
    pub unavailable: Option<&'static str>,
    /// Runs on Ollama: no key, and its models are whatever has been pulled.
    pub local: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider { id: "claude", name: "Claude", key: Some(secrets::CLAUDE_KEY), models: crate::claude::MODELS, background: false, unavailable: None, local: false },
    Provider {
        id: "chatgpt",
        name: "ChatGPT",
        key: Some(secrets::OPENAI_KEY),
        models: &[("gpt-5.6", "GPT-5.6 (best quality)"), ("gpt-5-mini", "GPT-5 mini (faster, cheaper)")],
        background: false,
        unavailable: None,
        local: false,
    },
    Provider {
        id: "grok",
        name: "Grok",
        key: Some(secrets::XAI_KEY),
        models: &[("grok-4.7", "Grok 4.7 (best quality)"), ("grok-4.3", "Grok 4.3 (cheaper)")],
        background: false,
        unavailable: None,
        local: false,
    },
    Provider { id: "cursor", name: "Cursor", key: Some(secrets::CURSOR_KEY), models: &[("auto", "Cursor's default model")], background: true, unavailable: None, local: false },
    Provider {
        id: "copilot",
        name: "Copilot",
        key: None,
        models: &[],
        background: false,
        unavailable: Some("Microsoft 365 Copilot can only be driven through Microsoft Graph with a Copilot license on your non-clinic account."),
        local: false,
    },
    Provider { id: "local", name: "Local AI", key: None, models: &[], background: false, unavailable: None, local: true },
];

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

#[derive(Serialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct ProviderStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub connected: bool,
    pub background: bool,
    pub unavailable: Option<&'static str>,
    pub models: Vec<ModelOption>,
    /// One line on how it's doing, for providers without a key (local models).
    pub note: Option<String>,
}

pub async fn statuses(secrets: &Secrets) -> Result<Vec<ProviderStatus>, String> {
    let mut out = Vec::new();
    for p in PROVIDERS {
        let mut models: Vec<ModelOption> =
            p.models.iter().map(|(id, label)| ModelOption { id: id.to_string(), label: label.to_string() }).collect();
        let mut note = None;
        let connected = if p.local {
            match crate::ollama::models().await {
                Ok(m) => {
                    note = Some(match m.len() {
                        0 => "Ollama is running, but no models are pulled yet.".to_string(),
                        1 => "1 model ready".to_string(),
                        n => format!("{n} models ready"),
                    });
                    models = m.into_iter().map(|(id, label)| ModelOption { id, label }).collect();
                    !models.is_empty()
                }
                Err(e) => {
                    note = Some(e);
                    false
                }
            }
        } else {
            match p.key {
                Some(k) => secrets.get(k)?.is_some(),
                None => false,
            }
        };
        out.push(ProviderStatus { id: p.id, name: p.name, connected, background: p.background, unavailable: p.unavailable, models, note });
    }
    Ok(out)
}

/// The agent the dashboard uses when you just ask a provider something.
pub fn quick_agent(p: &Provider) -> crate::db::AgentInput {
    crate::db::AgentInput {
        id: None,
        name: p.name.to_string(),
        description: format!("Ask {} anything from the dashboard.", p.name),
        system_prompt: "You are a helpful assistant answering James from his command center dashboard. \
                        Be concise and practical."
            .into(),
        model: p.models.first().map(|m| m.0).unwrap_or_default().to_string(),
        provider: p.id.to_string(),
        repo: String::new(),
        include_context: false,
        memory: !p.background,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn every_usable_provider_has_a_key_and_models() {
        for p in PROVIDERS {
            if p.unavailable.is_none() && !p.local {
                assert!(p.key.is_some(), "{}", p.id);
                assert!(!p.models.is_empty(), "{}", p.id);
            }
        }
        assert!(find("copilot").unwrap().unavailable.is_some());
        assert!(find("nope").is_none());
        assert!(find("local").unwrap().key.is_none());
    }

    #[tokio::test]
    async fn status_follows_saved_keys() {
        std::env::set_var("CC_OLLAMA_URL", "http://127.0.0.1:9");
        let s = Secrets::new(MemoryStore::default());
        s.set(secrets::XAI_KEY, "xai-test").unwrap();
        let st = statuses(&s).await.unwrap();
        let local = st.iter().find(|p| p.id == "local").unwrap();
        assert!(!local.connected && local.note.as_deref().unwrap().contains("Can't reach Ollama"));
        assert!(st.iter().find(|p| p.id == "grok").unwrap().connected);
        assert!(!st.iter().find(|p| p.id == "copilot").unwrap().connected);
    }
}
