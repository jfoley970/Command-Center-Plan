//! The Command Center backend, shared by the desktop app (local mode) and
//! `cc-server` (server mode). Everything that holds a key, polls an API, runs an
//! agent or keeps state lives here; the hosts only add a transport (Tauri IPC or
//! HTTP + WebSocket) and a way to show notifications.

pub mod atera;
pub mod claude;
pub mod commands;
pub mod cursor;
pub mod db;
pub mod mail;
pub mod openai;
pub mod providers;
pub mod secrets;
pub mod unifi;

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;

pub use commands::call;
pub use db::Db;
pub use secrets::Secrets;

/// Something the UI should hear about. `name` matches the event names the
/// frontend listens for ("mail-changed", "atera-changed", ...); "notify" asks
/// the client to show a desktop notification.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub name: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// How Microsoft sign-in reaches a browser.
pub enum SignIn {
    /// Local mode: open the system browser on this machine and catch the
    /// redirect on a loopback port.
    Browser(Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>),
    /// Server mode: the device code flow. The UI shows a code and a link that
    /// works from any device, and the server waits for Microsoft to confirm.
    DeviceCode,
}

pub struct Core {
    pub db: Db,
    pub secrets: Secrets,
    pub atera: atera::AteraState,
    pub unifi: unifi::UnifiState,
    pub sign_in: SignIn,
    pub data_dir: PathBuf,
    events: broadcast::Sender<Event>,
}

impl Core {
    pub fn open(data_dir: &Path, secrets: Secrets, sign_in: SignIn) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("Could not create {}: {e}", data_dir.display()))?;
        let db = Db::open(&data_dir.join("command-center.db")).map_err(|e| format!("Could not open the database: {e}"))?;
        db::mark_orphaned_runs(&db.0.lock().unwrap()).map_err(|e| e.to_string())?;
        let (events, _) = broadcast::channel(256);
        Ok(Arc::new(Self {
            db,
            secrets,
            atera: atera::AteraState::new(data_dir),
            unifi: unifi::UnifiState::new(data_dir),
            sign_in,
            data_dir: data_dir.to_path_buf(),
            events,
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn emit(&self, name: &str, payload: impl Serialize) {
        let payload = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
        // No listeners is fine: nobody has the app open.
        let _ = self.events.send(Event { name: name.to_string(), payload });
    }

    pub fn changed(&self, name: &str) {
        self.emit(name, ());
    }

    pub fn notify(&self, title: impl Into<String>, body: impl Into<String>) {
        self.emit("notify", Notice { title: title.into(), body: body.into() });
    }
}

/// Starts the reminder, mail, Atera, UniFi and Cursor loops. Call from inside a Tokio runtime.
pub fn start_background(core: &Arc<Core>) {
    tokio::spawn(reminder_loop(core.clone()));
    tokio::spawn(mail::sync_loop(core.clone()));
    tokio::spawn(atera::poll_loop(core.clone()));
    tokio::spawn(unifi::poll_loop(core.clone()));
    tokio::spawn(cursor::poll_loop(core.clone()));
}

/// Checks for due reminders every 15 seconds.
async fn reminder_loop(core: Arc<Core>) {
    loop {
        let due = db::take_due_reminders(&core.db.0.lock().unwrap()).unwrap_or_else(|e| {
            eprintln!("reminder check failed: {e}");
            vec![]
        });
        for r in &due {
            core.notify("Reminder", &r.title);
        }
        if !due.is_empty() {
            core.emit("reminders-fired", &due);
        }
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }
}
