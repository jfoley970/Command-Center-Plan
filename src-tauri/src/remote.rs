//! Server mode for the desktop app. Instead of running the backend in process,
//! the desktop app sends every command to a Command Center server and relays
//! the server's events, so it shows exactly what the web app shows: the same
//! data, the same connectors, the same keys (which stay on the server). The
//! desktop shell keeps what only it can do: tray, global shortcut and OS
//! notifications.
//!
//! The server sits behind Caddy and Authentik, so the app signs in like a
//! script would: with an Authentik app password for a user (HTTP Basic, which
//! Authentik's proxy provider accepts in place of a browser session), or with
//! cc-server's own `CC_API_TOKEN` (Bearer) where there is no proxy.

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

/// The keychain entry holding the server sign-in. Never written to a file.
pub const TOKEN_SECRET: &str = "command-center-server-token";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Local,
    Server,
}

/// `server.json` in the app's data folder. Holds no secrets.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
}

impl Config {
    fn path(dir: &Path) -> PathBuf {
        dir.join("server.json")
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path(dir), json).map_err(|e| format!("Could not save the server setting: {e}"))
    }
}

/// Checks and tidies a server address. HTTPS is required, except on this
/// machine for development, because the sign-in travels with every request.
pub fn normalize_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(raw).map_err(|_| "Enter the server's address, like https://cc.example.com.".to_string())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        _ => return Err("Use an https:// address, so your sign-in is never sent in the clear.".into()),
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err("Enter just the server's address, like https://cc.example.com.".into());
    }
    Ok(raw.to_string())
}

/// `Basic user:token` when there is a username (Authentik app password), else `Bearer token`.
pub fn auth_header(username: &str, token: &str) -> String {
    let (username, token) = (username.trim(), token.trim());
    if username.is_empty() {
        format!("Bearer {token}")
    } else {
        format!("Basic {}", STANDARD.encode(format!("{username}:{token}")))
    }
}

#[derive(Default)]
struct Status {
    connected: bool,
    error: Option<String>,
}

pub struct Remote {
    pub base: String,
    auth: String,
    http: reqwest::Client,
    status: Mutex<Status>,
}

impl Remote {
    pub fn new(base: &str, username: &str, token: &str) -> Result<Arc<Self>, String> {
        let http = reqwest::Client::builder()
            // A redirect means the sign-in proxy wants a browser login: the
            // app password was not accepted. Report that instead of following it.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            base: normalize_url(base)?,
            auth: auth_header(username, token),
            http,
            status: Mutex::new(Status::default()),
        }))
    }

    pub fn connected(&self) -> (bool, Option<String>) {
        let s = self.status.lock().unwrap();
        (s.connected, s.error.clone())
    }

    fn set_status(&self, connected: bool, error: Option<String>) {
        *self.status.lock().unwrap() = Status { connected, error };
    }

    /// Runs one command on the server, exactly as the web app does.
    pub async fn call(&self, command: &str, args: Value) -> Result<Value, String> {
        let resp = self
            .http
            .post(format!("{}/api/call/{command}", self.base))
            .header(reqwest::header::AUTHORIZATION, &self.auth)
            .json(&args)
            .send()
            .await
            .map_err(|e| format!("Could not reach the Command Center server: {e}"))?;
        let status = resp.status();
        if status.is_redirection() || status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(signed_out());
        }
        let body: Option<Value> = resp.json().await.ok();
        if status.is_success() {
            return body.ok_or_else(|| "The server sent something that isn't Command Center data. Check the address.".into());
        }
        Err(body
            .as_ref()
            .and_then(|b| b["error"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("The server returned {status}.")))
    }

    /// Streams the server's events into the window and OS notifications, and
    /// reconnects with backoff. After a reconnect the UI reloads ("resync").
    pub fn follow_events(self: &Arc<Self>, app: AppHandle) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut retry = 1u64;
            let mut first = true;
            loop {
                match me.stream(&app, first).await {
                    Ok(()) => retry = 1,
                    Err(e) => {
                        me.set_status(false, Some(e));
                        retry = (retry * 2).min(30);
                    }
                }
                first = false;
                let _ = app.emit("server-status", ());
                tokio::time::sleep(Duration::from_secs(retry)).await;
            }
        });
    }

    async fn stream(&self, app: &AppHandle, first: bool) -> Result<(), String> {
        let ws_url = format!("{}/api/events", self.base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1));
        let mut req = ws_url.into_client_request().map_err(|e| e.to_string())?;
        req.headers_mut()
            .insert("Authorization", HeaderValue::from_str(&self.auth).map_err(|_| "The sign-in has characters that can't be sent.".to_string())?);
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) if r.status().is_redirection() || r.status().as_u16() == 401 => signed_out(),
            e => format!("Lost the connection to the server: {e}"),
        })?;
        self.set_status(true, None);
        let _ = app.emit("server-status", ());
        if !first {
            let _ = app.emit("resync", ());
        }
        while let Some(msg) = ws.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t,
                Ok(Message::Close(_)) => break,
                Ok(_) => continue,
                Err(e) => return Err(format!("Lost the connection to the server: {e}")),
            };
            let Ok(ev) = serde_json::from_str::<Value>(&text) else { continue };
            let name = ev["name"].as_str().unwrap_or_default();
            if name == "notify" {
                let title = ev["payload"]["title"].as_str().unwrap_or("Command Center");
                let body = ev["payload"]["body"].as_str().unwrap_or_default();
                let _ = app.notification().builder().title(title).body(body).show();
            } else if !name.is_empty() {
                let _ = app.emit(name, ev["payload"].clone());
            }
        }
        self.set_status(false, Some("Lost the connection to the server. Reconnecting…".into()));
        Ok(())
    }
}

fn signed_out() -> String {
    "The server didn't accept this app's sign-in. Check the username and app password in Settings > Command Center server.".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_must_be_https_except_on_this_machine() {
        assert_eq!(normalize_url(" https://cc.example.com/ ").unwrap(), "https://cc.example.com");
        assert_eq!(normalize_url("http://127.0.0.1:8484").unwrap(), "http://127.0.0.1:8484");
        assert!(normalize_url("http://cc.example.com").is_err());
        assert!(normalize_url("cc.example.com").is_err());
        assert!(normalize_url("https://user:pw@cc.example.com").is_err());
        assert!(normalize_url("https://cc.example.com/?x=1").is_err());
    }

    #[test]
    fn basic_with_a_username_bearer_without() {
        assert_eq!(auth_header("james", "pw"), "Basic amFtZXM6cHc=");
        assert_eq!(auth_header("", " tok "), "Bearer tok");
    }

    #[test]
    fn config_round_trips_and_defaults_to_local() {
        let dir = std::env::temp_dir().join(format!("cc-remote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&dir).mode, Mode::Local);
        let c = Config { mode: Mode::Server, url: "https://cc.example.com".into(), username: "james".into() };
        c.save(&dir).unwrap();
        assert_eq!(Config::load(&dir), c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
