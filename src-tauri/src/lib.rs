//! The desktop shell. In local mode it runs the shared backend (`cc-core`) in
//! process; the UI reaches it through one `call` command, the same API the
//! server exposes over HTTP. The shell adds what only a desktop app can do:
//! the tray icon, the global shortcut and OS notifications.

use cc_core::{Core, Secrets, SignIn};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

struct Backend(Arc<Core>);

#[tauri::command]
async fn call(backend: State<'_, Backend>, command: String, args: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    cc_core::call(&backend.0, &command, args.unwrap_or_default()).await
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
            let bg = core.clone();
            tauri::async_runtime::spawn(async move { cc_core::start_background(&bg) });
            app.manage(Backend(core));

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
        .invoke_handler(tauri::generate_handler![call])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
