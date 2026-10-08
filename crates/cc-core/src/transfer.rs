//! Moving connectors from one Command Center to another, normally from the
//! desktop app's local data to the server, so the desktop app and the web app
//! can share one set of connections.
//!
//! Exporting reads keys out of the secret store, so it is a Rust function only:
//! no command returns a key, over IPC or HTTP. Importing is the
//! `import_connectors` command, which the server accepts like any other call
//! (behind the same sign-in), and it only writes the secret names and settings
//! listed here.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::secrets::{self, ATERA_KEY, CLAUDE_KEY, CURSOR_KEY, OPENAI_KEY, UNIFI_KEY, XAI_KEY};
use crate::{atera, db, unifi, Core};

/// A connector that can be moved, and the secret it is keyed by.
struct Kind {
    id: &'static str,
    name: &'static str,
    secret: Option<&'static str>,
}

/// Every connector that holds something worth moving. A new connector with a
/// key adds a line here (and to `Secrets::env_name` if it has an env var).
const KINDS: &[Kind] = &[
    Kind { id: "claude", name: "Claude", secret: Some(CLAUDE_KEY) },
    Kind { id: "atera", name: "Atera", secret: Some(ATERA_KEY) },
    Kind { id: "unifi", name: "UniFi Site Manager", secret: Some(UNIFI_KEY) },
    Kind { id: "microsoft365", name: "Microsoft 365", secret: None },
    Kind { id: "cursor", name: "Cursor", secret: Some(CURSOR_KEY) },
    Kind { id: "chatgpt", name: "ChatGPT", secret: Some(OPENAI_KEY) },
    Kind { id: "grok", name: "Grok", secret: Some(XAI_KEY) },
];

/// What a Command Center has set up, without any keys. Used to show which
/// connectors are on each side before moving them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConnectorSummary {
    pub id: String,
    pub name: String,
    pub configured: bool,
    /// One line, such as "2 hidden customers" or the inbox addresses.
    pub detail: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MailAccountBundle {
    pub email: String,
    pub display_name: String,
    pub client_id: String,
    pub tenant_id: String,
    pub refresh_token: String,
}

/// Everything one connector needs on the other side.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorBundle {
    pub id: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub hidden_customers: Vec<atera::HiddenCustomer>,
    #[serde(default)]
    pub hidden_sites: Vec<unifi::HiddenSite>,
    #[serde(default)]
    pub mail_client_id: Option<String>,
    #[serde(default)]
    pub mail_tenant_id: Option<String>,
    #[serde(default)]
    pub mail_accounts: Vec<MailAccountBundle>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImportResult {
    pub id: String,
    pub ok: bool,
    pub detail: String,
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

pub fn summary(core: &Core) -> Result<Vec<ConnectorSummary>, String> {
    let conn = core.db.0.lock().unwrap();
    let mut out = vec![];
    for k in KINDS {
        let (configured, detail) = match k.id {
            "microsoft365" => {
                let accounts = db::list_accounts(&conn).map_err(|e| e.to_string())?;
                let emails: Vec<String> = accounts.into_iter().map(|a| a.email).collect();
                (!emails.is_empty(), if emails.is_empty() { "No inboxes".into() } else { emails.join(", ") })
            }
            _ => {
                let has = core.secrets.get(k.secret.unwrap())?.is_some();
                let extra = match k.id {
                    "atera" => core.atera.snapshot(&core.secrets)?.hidden_customers.len(),
                    "unifi" => core.unifi.snapshot(&core.secrets)?.hidden_sites.len(),
                    _ => 0,
                };
                let what = if k.id == "atera" { "hidden customer" } else { "hidden site" };
                let detail = match (has, extra) {
                    (false, _) => "Not set up".to_string(),
                    (true, 0) => "Key saved".to_string(),
                    (true, n) => format!("Key saved · {}", plural(n, what)),
                };
                (has, detail)
            }
        };
        out.push(ConnectorSummary { id: k.id.into(), name: k.name.into(), configured, detail });
    }
    Ok(out)
}

/// Gathers the chosen connectors, keys included. Never expose this as a command.
pub fn export(core: &Core, ids: &[String]) -> Result<Vec<ConnectorBundle>, String> {
    let mut out = vec![];
    for id in ids {
        let kind = KINDS.iter().find(|k| k.id == id).ok_or_else(|| format!("Unknown connector: {id}"))?;
        let mut b = ConnectorBundle { id: kind.id.into(), ..Default::default() };
        if let Some(name) = kind.secret {
            b.key = core.secrets.get(name)?;
            if b.key.is_none() {
                return Err(format!("{} isn't set up in this app, so there is nothing to move.", kind.name));
            }
        }
        match kind.id {
            "atera" => b.hidden_customers = core.atera.snapshot(&core.secrets)?.hidden_customers,
            "unifi" => b.hidden_sites = core.unifi.snapshot(&core.secrets)?.hidden_sites,
            "microsoft365" => {
                let conn = core.db.0.lock().unwrap();
                b.mail_client_id = db::get_setting(&conn, "ms_client_id").map_err(|e| e.to_string())?;
                b.mail_tenant_id = db::get_setting(&conn, "ms_tenant_id").map_err(|e| e.to_string())?;
                for a in db::list_accounts(&conn).map_err(|e| e.to_string())? {
                    // An inbox without a saved sign-in has to be connected again anyway.
                    let Some(token) = core.secrets.get(&secrets::mail_token_name(&a.email))? else { continue };
                    b.mail_accounts.push(MailAccountBundle {
                        email: a.email,
                        display_name: a.display_name,
                        client_id: a.client_id,
                        tenant_id: a.tenant_id,
                        refresh_token: token,
                    });
                }
                if b.mail_accounts.is_empty() {
                    return Err("No Microsoft 365 inbox is signed in here, so there is nothing to move.".into());
                }
            }
            _ => {}
        }
        out.push(b);
    }
    Ok(out)
}

/// Saves connectors sent from another Command Center, then refreshes them so
/// the result shows whether the keys work from here.
pub async fn import(core: &Arc<Core>, bundles: Vec<ConnectorBundle>) -> Result<Vec<ImportResult>, String> {
    let mut results = vec![];
    for b in bundles {
        let Some(kind) = KINDS.iter().find(|k| k.id == b.id) else {
            results.push(ImportResult { id: b.id.clone(), ok: false, detail: "This server doesn't know that connector yet.".into() });
            continue;
        };
        if let Some(name) = kind.secret {
            match b.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
                Some(key) => core.secrets.set(name, key)?,
                None => {
                    results.push(ImportResult { id: b.id, ok: false, detail: "No key was sent.".into() });
                    continue;
                }
            }
        }
        let outcome: Result<String, String> = match kind.id {
            "claude" | "cursor" | "chatgpt" | "grok" => Ok("Key saved".into()),
            "atera" => {
                for c in b.hidden_customers {
                    core.atera.set_hidden(c, true)?;
                }
                atera::refresh(core).await.map(|s| plural(s.alerts.len(), "open alert"))
            }
            "unifi" => {
                for s in b.hidden_sites {
                    core.unifi.set_hidden(s, true)?;
                }
                unifi::refresh(core).await.map(|s| plural(s.sites.len(), "site"))
            }
            "microsoft365" => import_mail(core, &b),
            _ => Ok(String::new()),
        };
        results.push(match outcome {
            Ok(detail) => ImportResult { id: b.id, ok: true, detail },
            // The key is saved either way; the error says why it doesn't work yet.
            Err(detail) => ImportResult { id: b.id, ok: false, detail },
        });
    }
    for name in ["atera-changed", "unifi-changed", "mail-changed", "providers-changed", "connectors-changed"] {
        core.changed(name);
    }
    Ok(results)
}

fn import_mail(core: &Core, b: &ConnectorBundle) -> Result<String, String> {
    let conn = core.db.0.lock().unwrap();
    if let Some(v) = b.mail_client_id.as_deref().filter(|v| !v.is_empty()) {
        db::set_setting(&conn, "ms_client_id", v).map_err(|e| e.to_string())?;
    }
    if let Some(v) = b.mail_tenant_id.as_deref().filter(|v| !v.is_empty()) {
        db::set_setting(&conn, "ms_tenant_id", v).map_err(|e| e.to_string())?;
    }
    let mut moved = vec![];
    for a in &b.mail_accounts {
        let email = a.email.trim();
        if email.is_empty() || a.refresh_token.trim().is_empty() || a.client_id.trim().is_empty() {
            continue;
        }
        core.secrets.set(&secrets::mail_token_name(email), &a.refresh_token)?;
        db::upsert_account(&conn, email, &a.display_name, &a.client_id, &a.tenant_id).map_err(|e| e.to_string())?;
        moved.push(email.to_string());
    }
    if moved.is_empty() {
        return Err("No inbox came with a sign-in.".into());
    }
    // The mail loop picks new accounts up on its next pass.
    Ok(format!("{} moved: {}", plural(moved.len(), "inbox"), moved.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, Secrets};
    use crate::SignIn;

    fn core() -> Arc<Core> {
        let dir = std::env::temp_dir().join(format!("cc-transfer-{}", rand::random::<u64>()));
        Core::open(&dir, Secrets::new(MemoryStore::default()), SignIn::DeviceCode).unwrap()
    }

    #[tokio::test]
    async fn mail_and_claude_move_with_their_secrets() {
        let (from, to) = (core(), core());
        from.secrets.set(CLAUDE_KEY, "sk-moved").unwrap();
        {
            let c = from.db.0.lock().unwrap();
            db::set_setting(&c, "ms_client_id", "app-1").unwrap();
            db::upsert_account(&c, "ops@example.com", "Ops", "app-1", "common").unwrap();
            db::upsert_account(&c, "nosignin@example.com", "", "app-1", "common").unwrap();
        }
        from.secrets.set(&secrets::mail_token_name("ops@example.com"), "rt-123").unwrap();

        let bundles = export(&from, &["claude".into(), "microsoft365".into()]).unwrap();
        assert_eq!(bundles[1].mail_accounts.len(), 1, "inboxes without a sign-in stay behind");

        // Only meaningful when the developer has no key in the environment.
        let env_key = std::env::var("ANTHROPIC_API_KEY").is_ok();
        let results = import(&to, bundles).await.unwrap();
        assert!(results.iter().all(|r| r.ok), "{results:?}");
        if !env_key {
            assert_eq!(to.secrets.get(CLAUDE_KEY).unwrap().as_deref(), Some("sk-moved"));
        }
        assert_eq!(to.secrets.get(&secrets::mail_token_name("ops@example.com")).unwrap().as_deref(), Some("rt-123"));
        let c = to.db.0.lock().unwrap();
        assert_eq!(db::list_accounts(&c).unwrap()[0].email, "ops@example.com");
        assert_eq!(db::get_setting(&c, "ms_client_id").unwrap().as_deref(), Some("app-1"));
        drop(c);

        let s = summary(&to).unwrap();
        assert!(s.iter().find(|c| c.id == "microsoft365").unwrap().configured);
        for c in [from, to] {
            std::fs::remove_dir_all(&c.data_dir).unwrap();
        }
    }

    #[tokio::test]
    async fn export_refuses_connectors_that_are_not_set_up() {
        let from = core();
        if std::env::var("ATERA_API_KEY").is_err() {
            assert!(export(&from, &["atera".into()]).unwrap_err().contains("isn't set up"));
        }
        assert!(export(&from, &["microsoft365".into()]).is_err());
        assert!(export(&from, &["nope".into()]).unwrap_err().contains("Unknown connector"));
        std::fs::remove_dir_all(&from.data_dir).unwrap();
    }

    #[tokio::test]
    async fn import_skips_unknown_connectors_and_empty_keys() {
        let to = core();
        let r = import(
            &to,
            vec![
                ConnectorBundle { id: "pager".into(), ..Default::default() },
                ConnectorBundle { id: "claude".into(), key: Some("  ".into()), ..Default::default() },
            ],
        )
        .await
        .unwrap();
        assert!(r.iter().all(|r| !r.ok));
        std::fs::remove_dir_all(&to.data_dir).unwrap();
    }
}
