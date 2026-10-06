//! Every operation the UI can ask for, by name. The desktop app forwards Tauri
//! `invoke` calls here and the server forwards `POST /api/call/<name>`, so both
//! hosts expose exactly the same API. Argument names are camelCase, matching
//! what the frontend has always sent to Tauri.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::secrets::CLAUDE_KEY;
use crate::{atera, claude, db, mail, pomodoro, transfer, unifi, Core};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn args<T: DeserializeOwned>(v: Value) -> CmdResult<T> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_json::from_value(v).map_err(|e| format!("Bad arguments: {e}"))
}

fn ok<T: Serialize>(v: T) -> CmdResult<Value> {
    serde_json::to_value(v).map_err(err)
}

#[derive(Deserialize)]
struct Id {
    id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountId {
    account_id: i64,
}

#[derive(Deserialize)]
struct Key {
    key: String,
}

#[derive(Serialize)]
struct ModelOption {
    id: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
struct MailSetup {
    client_id: String,
    tenant_id: String,
}

/// Runs one named command with JSON arguments and returns its JSON result.
pub async fn call(core: &Arc<Core>, command: &str, a: Value) -> CmdResult<Value> {
    let conn = || core.db.0.lock().unwrap();
    match command {
        // ---------- Todos ----------
        "list_todos" => ok(db::list_todos(&conn()).map_err(err)?),
        "add_todo" => {
            #[derive(Deserialize)]
            struct A {
                todo: db::NewTodo,
            }
            let A { todo } = args(a)?;
            if todo.title.trim().is_empty() {
                return Err("A todo needs a title.".into());
            }
            ok(db::add_todo(&conn(), todo).map_err(err)?)
        }
        "set_todo_done" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                done: bool,
            }
            let A { id, done } = args(a)?;
            ok(db::set_todo_done(&conn(), id, done).map_err(err)?)
        }
        "set_todo_priority" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                priority: i64,
            }
            let A { id, priority } = args(a)?;
            ok(db::set_todo_priority(&conn(), id, priority).map_err(err)?)
        }
        "delete_todo" => ok(db::delete_todo(&conn(), args::<Id>(a)?.id).map_err(err)?),

        // ---------- Reminders ----------
        "list_reminders" => ok(db::list_reminders(&conn()).map_err(err)?),
        "add_reminder" => {
            #[derive(Deserialize)]
            struct A {
                reminder: db::NewReminder,
            }
            let A { reminder } = args(a)?;
            if reminder.title.trim().is_empty() {
                return Err("A reminder needs a title.".into());
            }
            let r = db::add_reminder(&conn(), reminder)?;
            core.changed("reminders-changed");
            ok(r)
        }
        "delete_reminder" => {
            db::delete_reminder(&conn(), args::<Id>(a)?.id).map_err(err)?;
            core.changed("reminders-changed");
            ok(())
        }
        "snooze_reminder" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                minutes: i64,
            }
            let A { id, minutes } = args(a)?;
            db::snooze_reminder(&conn(), id, minutes.max(1)).map_err(err)?;
            core.changed("reminders-changed");
            ok(())
        }
        "reschedule_reminder" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                id: i64,
                remind_at: String,
            }
            let A { id, remind_at } = args(a)?;
            let r = db::reschedule_reminder(&conn(), id, &remind_at)?;
            core.changed("reminders-changed");
            ok(r)
        }
        "extend_reminder" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                minutes: i64,
            }
            let A { id, minutes } = args(a)?;
            let r = db::extend_reminder(&conn(), id, minutes.clamp(1, 60 * 24 * 30))?;
            core.changed("reminders-changed");
            ok(r)
        }

        // ---------- Pomodoro ----------
        "pomodoro_get" => ok(pomodoro::get(core)),
        "pomodoro_start" => {
            #[derive(Deserialize)]
            struct A {
                phase: pomodoro::Phase,
                minutes: Option<i64>,
            }
            let A { phase, minutes } = args(a)?;
            ok(pomodoro::start(core, phase, minutes))
        }
        "pomodoro_pause" => ok(pomodoro::pause(core)),
        "pomodoro_resume" => ok(pomodoro::resume(core)),
        "pomodoro_reset" => ok(pomodoro::reset(core)),
        "pomodoro_dismiss" => ok(pomodoro::dismiss(core)),

        // ---------- Projects ----------
        "list_projects" => ok(db::list_projects(&conn()).map_err(err)?),
        "save_project" => {
            #[derive(Deserialize)]
            struct A {
                project: db::ProjectInput,
            }
            let A { project } = args(a)?;
            if project.name.trim().is_empty() {
                return Err("A project needs a name.".into());
            }
            ok(db::save_project(&conn(), project).map_err(err)?)
        }
        "delete_project" => ok(db::delete_project(&conn(), args::<Id>(a)?.id).map_err(err)?),

        // ---------- Mail ----------
        "get_mail_setup" => {
            let c = conn();
            ok(MailSetup {
                client_id: db::get_setting(&c, "ms_client_id").map_err(err)?.unwrap_or_default(),
                tenant_id: db::get_setting(&c, "ms_tenant_id").map_err(err)?.unwrap_or_default(),
            })
        }
        "connect_microsoft" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                client_id: String,
                tenant_id: String,
            }
            let A { client_id, tenant_id } = args(a)?;
            ok(mail::connect(core, &client_id, &tenant_id).await?)
        }
        "list_mail_accounts" => ok(db::list_accounts(&conn()).map_err(err)?),
        "list_emails" => ok(db::list_emails(&conn(), args::<AccountId>(a)?.account_id, 50).map_err(err)?),
        "sync_mail" => {
            let result = mail::sync_account(core, args::<AccountId>(a)?.account_id).await;
            core.changed("mail-changed");
            ok(result?)
        }
        "disconnect_mail" => {
            let id = args::<AccountId>(a)?.account_id;
            let c = conn();
            if let Some(acct) = db::get_account(&c, id).map_err(err)? {
                core.secrets.delete(&crate::secrets::mail_token_name(&acct.email))?;
            }
            ok(db::delete_account(&c, id).map_err(err)?)
        }
        "list_flag_links" => ok(db::list_flag_links(&conn()).map_err(err)?),
        "list_suggestions" => ok(db::list_pending_suggestions(&conn()).map_err(err)?),
        "accept_suggestion" => {
            #[derive(Deserialize)]
            struct A {
                suggestion: db::AcceptSuggestion,
            }
            ok(db::accept_suggestion(&conn(), args::<A>(a)?.suggestion)?)
        }
        "dismiss_suggestion" => ok(db::dismiss_suggestion(&conn(), args::<Id>(a)?.id).map_err(err)?),

        // ---------- Agents ----------
        "list_models" => ok(claude::MODELS.iter().map(|(id, label)| ModelOption { id, label }).collect::<Vec<_>>()),
        "list_agents" => ok(db::list_agents(&conn()).map_err(err)?),
        "save_agent" => {
            #[derive(Deserialize)]
            struct A {
                agent: db::AgentInput,
            }
            let A { agent } = args(a)?;
            if agent.name.trim().is_empty() || agent.system_prompt.trim().is_empty() {
                return Err("An agent needs a name and instructions.".into());
            }
            ok(db::save_agent(&conn(), agent).map_err(err)?)
        }
        "delete_agent" => ok(db::delete_agent(&conn(), args::<Id>(a)?.id).map_err(err)?),
        "list_runs" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: Option<i64>,
                limit: Option<i64>,
            }
            let A { agent_id, limit } = args(a)?;
            ok(db::list_runs(&conn(), agent_id, limit.unwrap_or(50)).map_err(err)?)
        }
        "run_agent" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: i64,
                input: String,
            }
            let A { agent_id, input } = args(a)?;
            ok(run_agent(core, agent_id, input).await?)
        }

        // ---------- Atera ----------
        "atera_alerts" => ok(core.atera.snapshot(&core.secrets)?),
        "atera_refresh" => ok(atera::refresh(core).await?),
        "atera_set_key" => {
            core.secrets.set(crate::secrets::ATERA_KEY, &args::<Key>(a)?.key)?;
            ok(atera::refresh(core).await?)
        }
        "atera_set_customer_hidden" => {
            #[derive(Deserialize)]
            struct A {
                customer: atera::HiddenCustomer,
                hidden: bool,
            }
            let A { customer, hidden } = args(a)?;
            core.atera.set_hidden(customer, hidden)?;
            core.changed("atera-changed");
            ok(())
        }

        // ---------- UniFi ----------
        "unifi_fleet" => ok(core.unifi.snapshot(&core.secrets)?),
        "unifi_refresh" => ok(unifi::refresh(core).await?),
        "unifi_set_key" => {
            core.secrets.set(crate::secrets::UNIFI_KEY, &args::<Key>(a)?.key)?;
            ok(unifi::refresh(core).await?)
        }
        "unifi_set_site_hidden" => {
            #[derive(Deserialize)]
            struct A {
                site: unifi::HiddenSite,
                hidden: bool,
            }
            let A { site, hidden } = args(a)?;
            core.unifi.set_hidden(site, hidden)?;
            core.changed("unifi-changed");
            ok(())
        }

        // ---------- Connectors moving between Command Centers ----------
        // There is deliberately no export command: keys never leave over the API.
        "connector_summary" => ok(transfer::summary(core)?),
        "import_connectors" => {
            #[derive(Deserialize)]
            struct A {
                connectors: Vec<transfer::ConnectorBundle>,
            }
            ok(transfer::import(core, args::<A>(a)?.connectors).await?)
        }

        // ---------- Layout and other view preferences ----------
        // Kept with the data rather than in the browser, so the desktop app and
        // every browser show the same dashboard.
        "ui_get" => {
            #[derive(Deserialize)]
            struct A {
                key: String,
            }
            let key = ui_key(&args::<A>(a)?.key)?;
            let value = db::get_setting(&conn(), &key).map_err(err)?;
            ok(value.and_then(|v| serde_json::from_str::<Value>(&v).ok()))
        }
        "ui_set" => {
            #[derive(Deserialize)]
            struct A {
                key: String,
                value: Value,
            }
            let A { key, value } = args(a)?;
            let key = ui_key(&key)?;
            let text = value.to_string();
            if text.len() > 64 * 1024 {
                return Err("That view setting is too large to save.".into());
            }
            db::set_setting(&conn(), &key, &text).map_err(err)?;
            core.emit("ui-changed", &key[3..]);
            ok(())
        }

        // ---------- Settings ----------
        "has_api_key" => ok(core.secrets.get(CLAUDE_KEY)?.is_some()),
        "set_api_key" => ok(core.secrets.set(CLAUDE_KEY, &args::<Key>(a)?.key)?),

        _ => Err(format!("Unknown command: {command}")),
    }
}

/// View settings share the settings table under a `ui:` prefix, so a UI key can
/// never overwrite a backend setting such as the Microsoft app registration.
fn ui_key(key: &str) -> CmdResult<String> {
    let ok = !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if ok {
        Ok(format!("ui:{key}"))
    } else {
        Err("Bad view setting name.".into())
    }
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

async fn run_agent(core: &Arc<Core>, agent_id: i64, input: String) -> CmdResult<db::AgentRun> {
    let api_key = core.secrets.get(CLAUDE_KEY)?.ok_or("Add your Claude API key in Settings before running an agent.")?;

    // Hold the lock only for database work, never across the network call.
    let (agent, run_id, context) = {
        let conn = core.db.0.lock().unwrap();
        let agent = db::get_agent(&conn, agent_id).map_err(err)?.ok_or("That agent no longer exists.")?;
        let input_label = if input.trim().is_empty() { "(no extra input)" } else { input.trim() };
        let run_id = db::start_run(&conn, agent_id, input_label).map_err(err)?;
        (agent, run_id, day_context(&conn)?)
    };
    core.changed("runs-changed");

    let user = if input.trim().is_empty() {
        context
    } else {
        format!("{context}\n\nRequest:\n{}", input.trim())
    };
    let result = claude::complete(&api_key, &agent.model, &agent.system_prompt, &user).await;

    let run = {
        let conn = core.db.0.lock().unwrap();
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
    core.changed("runs-changed");
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, Secrets};
    use crate::SignIn;
    use serde_json::json;

    fn core() -> Arc<Core> {
        let dir = std::env::temp_dir().join(format!("cc-core-{}", rand::random::<u64>()));
        Core::open(&dir, Secrets::new(MemoryStore::default()), SignIn::DeviceCode).unwrap()
    }

    #[tokio::test]
    async fn todo_round_trip_through_call() {
        let core = core();
        let todo = call(&core, "add_todo", json!({ "todo": { "title": "Buy cable", "priority": 1 } })).await.unwrap();
        assert_eq!(todo["title"], "Buy cable");
        let id = todo["id"].clone();
        call(&core, "set_todo_done", json!({ "id": id, "done": true })).await.unwrap();
        let list = call(&core, "list_todos", Value::Null).await.unwrap();
        assert!(list.as_array().unwrap().iter().any(|t| t["id"] == id && t["done"] == true));
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }

    #[tokio::test]
    async fn rejects_blank_titles_unknown_commands_and_bad_args() {
        let core = core();
        assert!(call(&core, "add_todo", json!({ "todo": { "title": "  " } })).await.is_err());
        assert!(call(&core, "no_such_thing", Value::Null).await.unwrap_err().contains("Unknown command"));
        assert!(call(&core, "delete_todo", json!({ "nope": 1 })).await.unwrap_err().contains("Bad arguments"));
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }

    #[tokio::test]
    async fn api_key_goes_to_the_secret_store() {
        let core = core();
        // Only meaningful when the developer has no key in the environment.
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            assert_eq!(call(&core, "has_api_key", Value::Null).await.unwrap(), json!(false));
            call(&core, "set_api_key", json!({ "key": "sk-test" })).await.unwrap();
            assert_eq!(call(&core, "has_api_key", Value::Null).await.unwrap(), json!(true));
        }
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }

    #[tokio::test]
    async fn view_settings_round_trip_and_stay_in_their_namespace() {
        let core = core();
        assert_eq!(call(&core, "ui_get", json!({ "key": "dashboard.layout" })).await.unwrap(), Value::Null);
        let layout = json!([{ "i": "todos", "x": 0, "y": 0, "w": 4, "h": 6 }]);
        call(&core, "ui_set", json!({ "key": "dashboard.layout", "value": layout })).await.unwrap();
        assert_eq!(call(&core, "ui_get", json!({ "key": "dashboard.layout" })).await.unwrap(), layout);
        assert!(call(&core, "ui_set", json!({ "key": "../ms_client_id", "value": 1 })).await.is_err());
        assert!(db::get_setting(&core.db.0.lock().unwrap(), "ms_client_id").unwrap().is_none());
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }

    #[tokio::test]
    async fn changes_reach_subscribers() {
        let core = core();
        let mut rx = core.subscribe();
        call(&core, "atera_set_customer_hidden", json!({ "customer": { "id": 3, "name": "Acme" }, "hidden": true }))
            .await
            .unwrap();
        assert_eq!(rx.recv().await.unwrap().name, "atera-changed");
        std::fs::remove_dir_all(&core.data_dir).unwrap();
    }
}
