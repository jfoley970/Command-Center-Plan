//! The desktop shell. In local mode it runs the shared backend (`cc-core`) in
//! process; the UI reaches it through one `call` command, the same API the
//! server exposes over HTTP. The shell adds what only a desktop app can do:
//! the tray icon, the global shortcut and OS notifications.
//!
//! In server mode (see `remote.rs`) the same `call` goes to a Command Center
//! server instead, so the desktop app and the web app are one app on one set
//! of data and connectors.

mod remote;

use cc_core::{transfer, Core, Secrets, SignIn};
use remote::{Config, Mode, Remote};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

struct Backend {
    /// This computer's own backend. In server mode it runs no background jobs
    /// and is only read to move connectors to the server.
    local: Arc<Core>,
    /// Set in server mode: where every command goes.
    remote: Option<Arc<Remote>>,
    config: Mutex<Config>,
    dir: PathBuf,
}

impl Backend {
    fn token(&self) -> Result<Option<String>, String> {
        self.local.secrets.get(remote::TOKEN_SECRET)
    }

    /// A client for the configured server, whichever mode the app is in.
    fn server(&self) -> Result<Arc<Remote>, String> {
        let c = self.config.lock().unwrap().clone();
        if c.url.is_empty() {
            return Err("Add the server's address first.".into());
        }
        let token = self.token()?.ok_or("Add the app password for the server first.")?;
        Remote::new(&c.url, &c.username, &token)
    }
}

#[tauri::command]
async fn call(backend: State<'_, Backend>, command: String, args: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let args = args.unwrap_or_default();
    match &backend.remote {
        Some(remote) => remote.call(&command, args).await,
        None => cc_core::call(&backend.local, &command, args).await,
    }
}

// ---------- Server mode settings (desktop only) ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopServer {
    mode: Mode,
    url: String,
    username: String,
    has_token: bool,
    connected: bool,
    error: Option<String>,
}

#[tauri::command]
fn desktop_server_get(backend: State<'_, Backend>) -> Result<DesktopServer, String> {
    let c = backend.config.lock().unwrap().clone();
    let (connected, error) = backend.remote.as_ref().map(|r| r.connected()).unwrap_or((false, None));
    // Keychain trouble shouldn't keep the window from opening.
    let has_token = backend.token().ok().flatten().is_some();
    Ok(DesktopServer { mode: c.mode, url: c.url, username: c.username, has_token, connected, error })
}

/// Saves the address and sign-in. An empty token keeps the saved one.
#[tauri::command]
fn desktop_server_save(backend: State<'_, Backend>, url: String, username: String, token: String) -> Result<(), String> {
    let url = remote::normalize_url(&url)?;
    if !token.trim().is_empty() {
        backend.local.secrets.set(remote::TOKEN_SECRET, &token)?;
    }
    let mut c = backend.config.lock().unwrap();
    c.url = url;
    c.username = username.trim().to_string();
    c.save(&backend.dir)
}

/// Connectors on each side, without keys, for the "move connectors" list.
#[derive(Serialize)]
struct ConnectorSides {
    local: Vec<transfer::ConnectorSummary>,
    server: Vec<transfer::ConnectorSummary>,
}

#[tauri::command]
async fn desktop_connectors(backend: State<'_, Backend>) -> Result<ConnectorSides, String> {
    let local = transfer::summary(&backend.local)?;
    let server = backend.server()?.call("connector_summary", serde_json::Value::Null).await?;
    let server = serde_json::from_value(server).map_err(|_| "The server is running an older Command Center. Update it first.".to_string())?;
    Ok(ConnectorSides { local, server })
}

/// Copies the chosen connectors, keys included, from this computer to the
/// server. Keys go straight from the keychain to the server over HTTPS; they
/// never pass through the page.
#[tauri::command]
async fn desktop_connectors_transfer(backend: State<'_, Backend>, ids: Vec<String>) -> Result<serde_json::Value, String> {
    if ids.is_empty() {
        return Err("Pick at least one connector.".into());
    }
    let bundles = transfer::export(&backend.local, &ids)?;
    let server = backend.server()?;
    server.call("import_connectors", serde_json::json!({ "connectors": bundles })).await
}

/// Switches between this computer's data and the server, then restarts.
#[tauri::command]
async fn desktop_server_use(app: AppHandle, backend: State<'_, Backend>, server: bool) -> Result<(), String> {
    if server {
        // Don't switch to a server the app can't sign in to.
        backend.server()?.call("connector_summary", serde_json::Value::Null).await?;
    }
    {
        let mut c = backend.config.lock().unwrap();
        c.mode = if server { Mode::Server } else { Mode::Local };
        c.save(&backend.dir)?;
    }
    app.restart();
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn palette_shortcut() -> Shortcut {
    #[cfg(target_os = "macos")]
    let mods = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let mods = Modifiers::CONTROL | Modifiers::SHIFT;
    Shortcut::new(Some(mods), Code::Space)
}

/// Relays backend events to the UI, and "notify" events to OS notifications.
fn forward_events(app: AppHandle, core: &Arc<Core>) {
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) if ev.name == "notify" => {
                    let title = ev.payload["title"].as_str().unwrap_or("Command Center");
                    let body = ev.payload["body"].as_str().unwrap_or_default();
                    let _ = app.notification().builder().title(title).body(body).show();
                }
                Ok(ev) => {
                    let _ = app.emit(&ev.name, ev.payload);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = app.emit("resync", ());
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if shortcut == &palette_shortcut() && event.state() == ShortcutState::Pressed {
                        show_main(app);
                        let _ = app.emit("open-palette", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let opener = app.handle().clone();
            let sign_in = SignIn::Browser(Box::new(move |url: &str| {
                use tauri_plugin_opener::OpenerExt;
                opener.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
            }));
            let core = Core::open(&dir, Secrets::new(cc_core::secrets::KeyringStore), sign_in)?;
            let config = Config::load(&dir);
            let remote = match config.mode {
                Mode::Local => None,
                Mode::Server => {
                    let token = core.secrets.get(remote::TOKEN_SECRET).ok().flatten().unwrap_or_default();
                    match Remote::new(&config.url, &config.username, &token) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            // A broken setting must not lock James out of the app.
                            eprintln!("server mode is set but unusable, staying local: {e}");
                            None
                        }
                    }
                }
            };
            match &remote {
                Some(r) => r.follow_events(app.handle().clone()),
                None => {
                    forward_events(app.handle().clone(), &core);
                    // Bring the window back after Microsoft sign-in finishes in the browser.
                    let mut rx = core.subscribe();
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        while let Ok(ev) = rx.recv().await {
                            if ev.name == "mail-connected" {
                                show_main(&handle);
                            }
                        }
                    });
                    // In server mode the server polls and reminds; doing it here too would double every notification.
                    let bg = core.clone();
                    tauri::async_runtime::spawn(async move { cc_core::start_background(&bg) });
                }
            }
            let config = Mutex::new(Config { mode: if remote.is_some() { Mode::Server } else { Mode::Local }, ..config });
            app.manage(Backend { local: core, remote, config, dir });

            let show = MenuItem::with_id(app, "show", "Open Command Center", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Command Center")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if let Err(e) = app.global_shortcut().register(palette_shortcut()) {
                // Another app may own the shortcut; the in-app Ctrl/Cmd+K still works.
                eprintln!("could not register global shortcut: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the app running in the tray so reminders still fire.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            call,
            desktop_server_get,
            desktop_server_save,
            desktop_connectors,
            desktop_connectors_transfer,
            desktop_server_use
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
