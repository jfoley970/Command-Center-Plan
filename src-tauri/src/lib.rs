mod atera;
mod claude;
mod db;
mod mail;
mod secrets;

use db::Db;
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------- Todos ----------

#[tauri::command]
fn list_todos(db: State<Db>) -> CmdResult<Vec<db::Todo>> {
    db::list_todos(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn add_todo(db: State<Db>, todo: db::NewTodo) -> CmdResult<db::Todo> {
    if todo.title.trim().is_empty() {
        return Err("A todo needs a title.".into());
    }
    db::add_todo(&db.0.lock().unwrap(), todo).map_err(err)
}

#[tauri::command]
fn set_todo_done(db: State<Db>, id: i64, done: bool) -> CmdResult<()> {
    db::set_todo_done(&db.0.lock().unwrap(), id, done).map_err(err)
}

#[tauri::command]
fn set_todo_priority(db: State<Db>, id: i64, priority: i64) -> CmdResult<()> {
    db::set_todo_priority(&db.0.lock().unwrap(), id, priority).map_err(err)
}

#[tauri::command]
fn delete_todo(db: State<Db>, id: i64) -> CmdResult<()> {
    db::delete_todo(&db.0.lock().unwrap(), id).map_err(err)
}

// ---------- Reminders ----------

#[tauri::command]
fn list_reminders(db: State<Db>) -> CmdResult<Vec<db::Reminder>> {
    db::list_reminders(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn add_reminder(db: State<Db>, reminder: db::NewReminder) -> CmdResult<db::Reminder> {
    if reminder.title.trim().is_empty() {
        return Err("A reminder needs a title.".into());
    }
    db::add_reminder(&db.0.lock().unwrap(), reminder)
}

#[tauri::command]
fn delete_reminder(db: State<Db>, id: i64) -> CmdResult<()> {
    db::delete_reminder(&db.0.lock().unwrap(), id).map_err(err)
}

#[tauri::command]
fn snooze_reminder(db: State<Db>, id: i64, minutes: i64) -> CmdResult<()> {
    db::snooze_reminder(&db.0.lock().unwrap(), id, minutes.max(1)).map_err(err)
}

// ---------- Projects ----------

#[tauri::command]
fn list_projects(db: State<Db>) -> CmdResult<Vec<db::Project>> {
    db::list_projects(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn save_project(db: State<Db>, project: db::ProjectInput) -> CmdResult<db::Project> {
    if project.name.trim().is_empty() {
        return Err("A project needs a name.".into());
    }
    db::save_project(&db.0.lock().unwrap(), project).map_err(err)
}

#[tauri::command]
fn delete_project(db: State<Db>, id: i64) -> CmdResult<()> {
    db::delete_project(&db.0.lock().unwrap(), id).map_err(err)
}

// ---------- Mail ----------

#[derive(Serialize)]
struct MailSetup {
    client_id: String,
    tenant_id: String,
}

#[tauri::command]
fn get_mail_setup(db: State<Db>) -> CmdResult<MailSetup> {
    let conn = db.0.lock().unwrap();
    Ok(MailSetup {
        client_id: db::get_setting(&conn, "ms_client_id").map_err(err)?.unwrap_or_default(),
        tenant_id: db::get_setting(&conn, "ms_tenant_id").map_err(err)?.unwrap_or_default(),
    })
}

/// Opens Microsoft sign-in in the browser and connects the inbox that signs in.
#[tauri::command]
async fn connect_microsoft(app: AppHandle, client_id: String, tenant_id: String) -> CmdResult<db::MailAccount> {
    use tauri_plugin_opener::OpenerExt;
    let (client_id, tenant_id) = (client_id.trim().to_string(), tenant_id.trim().to_string());
    if client_id.is_empty() || tenant_id.is_empty() {
        return Err("Enter the Application (client) ID and Directory (tenant) ID first.".into());
    }
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::set_setting(&conn, "ms_client_id", &client_id).map_err(err)?;
        db::set_setting(&conn, "ms_tenant_id", &tenant_id).map_err(err)?;
    }
    let pending = mail::begin_sign_in(&client_id, &tenant_id)?;
    app.opener().open_url(pending.auth_url.clone(), None::<&str>).map_err(|e| format!("Could not open the browser: {e}"))?;
    let signed_in = mail::finish_sign_in(pending, client_id.clone(), tenant_id.clone()).await?;
    secrets::set_mail_token(&signed_in.email, &signed_in.refresh_token)?;
    let account = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        let id = db::upsert_account(&conn, &signed_in.email, &signed_in.display_name, &client_id, &tenant_id).map_err(err)?;
        db::get_account(&conn, id).map_err(err)?.ok_or("The inbox could not be saved.")?
    };
    show_main(&app);
    let _ = app.emit("mail-changed", ());
    let sync_app = app.clone();
    let id = account.id;
    tauri::async_runtime::spawn(async move {
        let _ = mail::sync_account(&sync_app, id).await;
        let _ = sync_app.emit("mail-changed", ());
    });
    Ok(account)
}

#[tauri::command]
fn list_mail_accounts(db: State<Db>) -> CmdResult<Vec<db::MailAccount>> {
    db::list_accounts(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn list_emails(db: State<Db>, account_id: i64) -> CmdResult<Vec<db::Email>> {
    db::list_emails(&db.0.lock().unwrap(), account_id, 50).map_err(err)
}

#[tauri::command]
async fn sync_mail(app: AppHandle, account_id: i64) -> CmdResult<usize> {
    let result = mail::sync_account(&app, account_id).await;
    let _ = app.emit("mail-changed", ());
    result
}

#[tauri::command]
fn disconnect_mail(db: State<Db>, account_id: i64) -> CmdResult<()> {
    let conn = db.0.lock().unwrap();
    if let Some(a) = db::get_account(&conn, account_id).map_err(err)? {
        secrets::delete_mail_token(&a.email)?;
    }
    db::delete_account(&conn, account_id).map_err(err)
}

#[tauri::command]
fn list_flag_links(db: State<Db>) -> CmdResult<Vec<db::FlagLink>> {
    db::list_flag_links(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn list_suggestions(db: State<Db>) -> CmdResult<Vec<db::Suggestion>> {
    db::list_pending_suggestions(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn accept_suggestion(db: State<Db>, suggestion: db::AcceptSuggestion) -> CmdResult<()> {
    db::accept_suggestion(&db.0.lock().unwrap(), suggestion)
}

#[tauri::command]
fn dismiss_suggestion(db: State<Db>, id: i64) -> CmdResult<()> {
    db::dismiss_suggestion(&db.0.lock().unwrap(), id).map_err(err)
}

/// Syncs every connected inbox shortly after launch and then every 15 minutes.
fn start_mail_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            let ids: Vec<i64> = {
                let state = app.state::<Db>();
                let conn = state.0.lock().unwrap();
                db::list_accounts(&conn).map(|a| a.into_iter().map(|a| a.id).collect()).unwrap_or_default()
            };
            for id in ids {
                if let Err(e) = mail::sync_account(&app, id).await {
                    eprintln!("mail sync failed: {e}");
                }
            }
            let _ = app.emit("mail-changed", ());
            tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
        }
    });
}

// ---------- Agents ----------

#[derive(Serialize)]
struct ModelOption {
    id: &'static str,
    label: &'static str,
}

#[tauri::command]
fn list_models() -> Vec<ModelOption> {
    claude::MODELS.iter().map(|(id, label)| ModelOption { id, label }).collect()
}

#[tauri::command]
fn list_agents(db: State<Db>) -> CmdResult<Vec<db::Agent>> {
    db::list_agents(&db.0.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn save_agent(db: State<Db>, agent: db::AgentInput) -> CmdResult<db::Agent> {
    if agent.name.trim().is_empty() || agent.system_prompt.trim().is_empty() {
        return Err("An agent needs a name and instructions.".into());
    }
    db::save_agent(&db.0.lock().unwrap(), agent).map_err(err)
}

#[tauri::command]
fn delete_agent(db: State<Db>, id: i64) -> CmdResult<()> {
    db::delete_agent(&db.0.lock().unwrap(), id).map_err(err)
}

#[tauri::command]
fn list_runs(db: State<Db>, agent_id: Option<i64>, limit: Option<i64>) -> CmdResult<Vec<db::AgentRun>> {
    db::list_runs(&db.0.lock().unwrap(), agent_id, limit.unwrap_or(50)).map_err(err)
}

/// Gives agents the same picture of the day the dashboard shows.
fn day_context(conn: &rusqlite::Connection) -> CmdResult<String> {
    let todos = db::list_todos(conn).map_err(err)?;
    let reminders = db::list_reminders(conn).map_err(err)?;
    let projects = db::list_projects(conn).map_err(err)?;
    let project_name = |id: Option<i64>| {
        id.and_then(|id| projects.iter().find(|p| p.id == id))
            .map(|p| format!(" [project: {}]", p.name))
            .unwrap_or_default()
    };
    let mut s = format!("Current time: {}\n\nOpen todos:\n", chrono::Local::now().format("%A %Y-%m-%d %H:%M"));
    for t in todos.iter().filter(|t| !t.done) {
        let due = t.due_at.as_deref().map(|d| format!(" (due {d})")).unwrap_or_default();
        s.push_str(&format!("- [P{}] {}{}{}\n", t.priority, t.title, due, project_name(t.project_id)));
    }
    s.push_str("\nUpcoming reminders:\n");
    for r in reminders.iter().filter(|r| !r.fired) {
        s.push_str(&format!("- {} at {} (repeat: {})\n", r.title, r.remind_at, r.repeat));
    }
    Ok(s)
}

#[tauri::command]
async fn run_agent(app: AppHandle, agent_id: i64, input: String) -> CmdResult<db::AgentRun> {
    let api_key = secrets::get_api_key()?
        .ok_or("Add your Claude API key in Settings before running an agent.")?;

    // Hold the lock only for database work, never across the network call.
    let (agent, run_id, context) = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        let agent = db::get_agent(&conn, agent_id).map_err(err)?.ok_or("That agent no longer exists.")?;
        let input_label = if input.trim().is_empty() { "(no extra input)" } else { input.trim() };
        let run_id = db::start_run(&conn, agent_id, input_label).map_err(err)?;
        (agent, run_id, day_context(&conn)?)
    };
    let _ = app.emit("runs-changed", ());

    let user = if input.trim().is_empty() {
        context
    } else {
        format!("{context}\n\nRequest:\n{}", input.trim())
    };
    let result = claude::complete(&api_key, &agent.model, &agent.system_prompt, &user).await;

    let run = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        match &result {
            Ok(c) => {
                let status = if c.stop_reason == "refusal" { "refused" } else { "done" };
                db::finish_run(&conn, run_id, status, &c.text, &c.model, c.input_tokens, c.output_tokens)
            }
            Err(e) => db::finish_run(&conn, run_id, "error", e, &agent.model, 0, 0),
        }
        .map_err(err)?;
        db::list_runs(&conn, Some(agent_id), 1).map_err(err)?.remove(0)
    };
    let _ = app.emit("runs-changed", ());
    Ok(run)
}

// ---------- Atera ----------

#[tauri::command]
fn atera_alerts(atera: State<atera::AteraState>) -> CmdResult<atera::Snapshot> {
    atera.snapshot()
}

#[tauri::command]
async fn atera_refresh(app: AppHandle) -> CmdResult<atera::Snapshot> {
    atera::refresh(&app).await
}

#[tauri::command]
async fn atera_set_key(app: AppHandle, key: String) -> CmdResult<atera::Snapshot> {
    atera::set_key(&key)?;
    atera::refresh(&app).await
}

#[tauri::command]
fn atera_set_customer_hidden(
    app: AppHandle,
    atera: State<atera::AteraState>,
    customer: atera::HiddenCustomer,
    hidden: bool,
) -> CmdResult<()> {
    atera.set_hidden(customer, hidden)?;
    let _ = app.emit("atera-changed", ());
    Ok(())
}

// ---------- Settings ----------

#[tauri::command]
fn has_api_key() -> CmdResult<bool> {
    Ok(secrets::get_api_key()?.is_some())
}

#[tauri::command]
fn set_api_key(key: String) -> CmdResult<()> {
    secrets::set_api_key(&key)
}

// ---------- App shell ----------

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

/// Checks for due reminders every 15 seconds and shows OS notifications.
fn start_reminder_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let due = {
                let state = app.state::<Db>();
                let conn = state.0.lock().unwrap();
                db::take_due_reminders(&conn).unwrap_or_else(|e| {
                    eprintln!("reminder check failed: {e}");
                    vec![]
                })
            };
            for r in &due {
                let _ = app.notification().builder().title("Reminder").body(&r.title).show();
            }
            if !due.is_empty() {
                let _ = app.emit("reminders-fired", &due);
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
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
            std::fs::create_dir_all(&dir)?;
            let database = Db::open(&dir.join("command-center.db"))?;
            db::mark_orphaned_runs(&database.0.lock().unwrap())?;
            app.manage(database);
            app.manage(atera::AteraState::new(&dir));

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

            start_reminder_loop(app.handle().clone());
            start_mail_loop(app.handle().clone());
            atera::start_loop(app.handle().clone());
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
            list_todos,
            add_todo,
            set_todo_done,
            set_todo_priority,
            delete_todo,
            list_reminders,
            add_reminder,
            delete_reminder,
            snooze_reminder,
            list_projects,
            save_project,
            delete_project,
            get_mail_setup,
            connect_microsoft,
            list_mail_accounts,
            list_emails,
            sync_mail,
            disconnect_mail,
            list_flag_links,
            list_suggestions,
            accept_suggestion,
            dismiss_suggestion,
            list_models,
            list_agents,
            save_agent,
            delete_agent,
            list_runs,
            run_agent,
            has_api_key,
            set_api_key,
            atera_alerts,
            atera_refresh,
            atera_set_key,
            atera_set_customer_hidden,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
