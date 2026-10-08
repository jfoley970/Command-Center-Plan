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
}

pub const PROVIDERS: &[Provider] = &[
    Provider { id: "claude", name: "Claude", key: Some(secrets::CLAUDE_KEY), models: crate::claude::MODELS, background: false, unavailable: None },
    Provider {
        id: "chatgpt",
        name: "ChatGPT",
        key: Some(secrets::OPENAI_KEY),
        models: &[("gpt-5.6", "GPT-5.6 (best quality)"), ("gpt-5-mini", "GPT-5 mini (faster, cheaper)")],
        background: false,
        unavailable: None,
    },
    Provider {
        id: "grok",
        name: "Grok",
        key: Some(secrets::XAI_KEY),
        models: &[("grok-4.7", "Grok 4.7 (best quality)"), ("grok-4.3", "Grok 4.3 (cheaper)")],
        background: false,
        unavailable: None,
    },
    Provider { id: "cursor", name: "Cursor", key: Some(secrets::CURSOR_KEY), models: &[("auto", "Cursor's default model")], background: true, unavailable: None },
    Provider {
        id: "copilot",
        name: "Copilot",
        key: None,
        models: &[],
        background: false,
        unavailable: Some("Microsoft 365 Copilot can only be driven through Microsoft Graph with a Copilot license on your non-clinic account."),
    },
];

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

#[derive(Serialize)]
pub struct ModelOption {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Serialize)]
pub struct ProviderStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub connected: bool,
    pub background: bool,
    pub unavailable: Option<&'static str>,
    pub models: Vec<ModelOption>,
}

pub fn statuses(secrets: &Secrets) -> Result<Vec<ProviderStatus>, String> {
    PROVIDERS
        .iter()
        .map(|p| {
            let connected = match p.key {
                Some(k) => secrets.get(k)?.is_some(),
                None => false,
            };
            Ok(ProviderStatus {
                id: p.id,
                name: p.name,
                connected,
                background: p.background,
                unavailable: p.unavailable,
                models: p.models.iter().map(|(id, label)| ModelOption { id, label }).collect(),
            })
        })
        .collect()
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn every_usable_provider_has_a_key_and_models() {
        for p in PROVIDERS {
            if p.unavailable.is_none() {
                assert!(p.key.is_some(), "{}", p.id);
                assert!(!p.models.is_empty(), "{}", p.id);
            }
        }
        assert!(find("copilot").unwrap().unavailable.is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn status_follows_saved_keys() {
        let s = Secrets::new(MemoryStore::default());
        s.set(secrets::XAI_KEY, "xai-test").unwrap();
        let st = statuses(&s).unwrap();
        assert!(st.iter().find(|p| p.id == "grok").unwrap().connected);
        assert!(!st.iter().find(|p| p.id == "copilot").unwrap().connected);
    }
}
