//! Every operation the UI can ask for, by name. The desktop app forwards Tauri
//! `invoke` calls here and the server forwards `POST /api/call/<name>`, so both
//! hosts expose exactly the same API. Argument names are camelCase, matching
//! what the frontend has always sent to Tauri.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::secrets::CLAUDE_KEY;
use crate::{atera, claude, cursor, db, mail, openai, providers, unifi, Core};

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
#[serde(rename_all = "camelCase")]
struct RunId {
    run_id: i64,
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
        "set_todo_notes" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                notes: String,
            }
            let A { id, notes } = args(a)?;
            ok(db::set_todo_notes(&conn(), id, &notes).map_err(err)?)
        }
        "todo_email" => ok(db::todo_email(&conn(), args::<Id>(a)?.id).map_err(err)?),
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
            ok(db::add_reminder(&conn(), reminder)?)
        }
        "delete_reminder" => ok(db::delete_reminder(&conn(), args::<Id>(a)?.id).map_err(err)?),
        "snooze_reminder" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                minutes: i64,
            }
            let A { id, minutes } = args(a)?;
            ok(db::snooze_reminder(&conn(), id, minutes.max(1)).map_err(err)?)
        }

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
            let provider = providers::find(&agent.provider).ok_or("Unknown AI provider.")?;
            if let Some(why) = provider.unavailable {
                return Err(why.into());
            }
            if agent.name.trim().is_empty() {
                return Err("An agent needs a name.".into());
            }
            if provider.background {
                cursor::normalize_repo(&agent.repo)?;
            } else if agent.system_prompt.trim().is_empty() {
                return Err("An agent needs instructions.".into());
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
        "cursor_conversation" => {
            let (_, remote, key) = cursor_run(core, args::<RunId>(a)?.run_id)?;
            ok(cursor::conversation(&key, &remote).await?)
        }
        "cursor_followup" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                run_id: i64,
                text: String,
            }
            let A { run_id, text } = args(a)?;
            if text.trim().is_empty() {
                return Err("Type what Cursor should do next.".into());
            }
            let (run, remote, key) = cursor_run(core, run_id)?;
            cursor::followup(&key, &remote, text.trim()).await?;
            let id = db::start_followup_run(&conn(), run.agent_id, text.trim(), &remote, &run.link).map_err(err)?;
            core.changed("runs-changed");
            ok(db::get_run(&conn(), id).map_err(err)?)
        }
        "cursor_stop" => {
            let (run, remote, key) = cursor_run(core, args::<RunId>(a)?.run_id)?;
            cursor::stop(&key, &remote).await?;
            if run.status == "running" {
                db::finish_external_run(&conn(), run.id, "stopped", "You stopped this run.", "").map_err(err)?;
            }
            core.changed("runs-changed");
            ok(())
        }
        "list_providers" => ok(providers::statuses(&core.secrets)?),
        "set_provider_key" => {
            #[derive(Deserialize)]
            struct A {
                provider: String,
                key: String,
            }
            let A { provider, key } = args(a)?;
            let name = providers::find(&provider).and_then(|p| p.key).ok_or("That provider doesn't take a key.")?;
            core.secrets.set(name, &key)?;
            core.changed("providers-changed");
            ok(())
        }
        "ask_provider" => {
            // "chatgpt: ..." from the dashboard: use the provider's first agent,
            // making a plain one the first time.
            #[derive(Deserialize)]
            struct A {
                provider: String,
                input: String,
            }
            let A { provider, input } = args(a)?;
            let p = providers::find(&provider).ok_or("Unknown AI provider.")?;
            if let Some(why) = p.unavailable {
                return Err(why.into());
            }
            let agent_id = {
                let c = conn();
                match db::list_agents(&c).map_err(err)?.into_iter().find(|a| a.provider == p.id) {
                    Some(a) => a.id,
                    None if p.background => {
                        return Err(format!("Make a {} agent under Agents first, so it knows which repository to work on.", p.name))
                    }
                    None => db::save_agent(&c, providers::quick_agent(p)).map_err(err)?.id,
                }
            };
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

        // ---------- Settings ----------
        "has_api_key" => ok(core.secrets.get(CLAUDE_KEY)?.is_some()),
        "set_api_key" => ok(core.secrets.set(CLAUDE_KEY, &args::<Key>(a)?.key)?),

        _ => Err(format!("Unknown command: {command}")),
    }
}

/// A Cursor run, its remote agent id and the Cursor key.
fn cursor_run(core: &Arc<Core>, run_id: i64) -> CmdResult<(db::AgentRun, String, String)> {
    let run = db::get_run(&core.db.0.lock().unwrap(), run_id).map_err(err)?.ok_or("That run no longer exists.")?;
    let remote = run.external_id.clone().ok_or("This run isn't a Cursor agent.")?;
    let key = core.secrets.get(crate::secrets::CURSOR_KEY)?.ok_or("Add your Cursor key in Settings > Connections.")?;
    Ok((run, remote, key))
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
    let agent = db::get_agent(&core.db.0.lock().unwrap(), agent_id).map_err(err)?.ok_or("That agent no longer exists.")?;
    let provider = providers::find(&agent.provider).ok_or("This agent's AI provider isn't supported.")?;
    if let Some(why) = provider.unavailable {
        return Err(why.into());
    }
    let key_name = provider.key.ok_or("This provider doesn't take a key.")?;
    let api_key = core
        .secrets
        .get(key_name)?
        .ok_or_else(|| format!("Add your {} key in Settings > Connections before running this agent.", provider.name))?;
    if provider.background && input.trim().is_empty() && agent.system_prompt.trim().is_empty() {
        return Err(format!("Tell {} what to do first.", provider.name));
    }

    // Hold the lock only for database work, never across the network call.
    let (run_id, context) = {
        let conn = core.db.0.lock().unwrap();
        let input_label = if input.trim().is_empty() { "(no extra input)" } else { input.trim() };
        let run_id = db::start_run(&conn, agent_id, input_label).map_err(err)?;
        let context = if agent.include_context { day_context(&conn)? } else { String::new() };
        (run_id, context)
    };
    core.changed("runs-changed");

    let user = match (context.is_empty(), input.trim().is_empty()) {
        (true, _) => input.trim().to_string(),
        (false, true) => context,
        (false, false) => format!("{context}\n\nRequest:\n{}", input.trim()),
    };

    if provider.background {
        // Cursor works on its own; the poll loop records how it ends.
        let prompt = [agent.system_prompt.trim(), user.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n\n");
        let conn = || core.db.0.lock().unwrap();
        match cursor::launch(&api_key, &agent.repo, &prompt).await {
            Ok(l) => db::set_run_external(&conn(), run_id, &l.id, &l.link).map_err(err)?,
            Err(e) => db::finish_run(&conn(), run_id, "error", &e, &agent.model, 0, 0).map_err(err)?,
        }
        core.changed("runs-changed");
        return Ok(db::list_runs(&conn(), Some(agent_id), 1).map_err(err)?.remove(0));
    }

    let result = match provider.id {
        "chatgpt" => openai::chatgpt(&api_key, &agent.model, &agent.system_prompt, &user).await,
        "grok" => openai::grok(&api_key, &agent.model, &agent.system_prompt, &user).await,
        _ => claude::complete(&api_key, &agent.model, &agent.system_prompt, &user).await,
    };

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
    async fn providers_route_and_validate() {
        let core = core();
        let e = call(&core, "ask_provider", json!({ "provider": "copilot", "input": "hi" })).await.unwrap_err();
        assert!(e.contains("Copilot license"));
        let e = call(&core, "ask_provider", json!({ "provider": "cursor", "input": "fix it" })).await.unwrap_err();
        assert!(e.contains("repository"));
        let bad = json!({ "agent": { "name": "Bugfixer", "model": "auto", "provider": "cursor", "repo": "nope" } });
        assert!(call(&core, "save_agent", bad).await.is_err());
        let good = json!({ "agent": { "name": "Bugfixer", "model": "auto", "provider": "cursor", "repo": "jfoley970/Command-Center-Plan" } });
        assert_eq!(call(&core, "save_agent", good).await.unwrap()["provider"], "cursor");

        if std::env::var("XAI_API_KEY").is_err() {
            // No key: the quick Grok agent is made, then the run explains what's missing.
            let e = call(&core, "ask_provider", json!({ "provider": "grok", "input": "hi" })).await.unwrap_err();
            assert!(e.contains("Grok key"));
            let agents = call(&core, "list_agents", Value::Null).await.unwrap();
            let grok = agents.as_array().unwrap().iter().find(|a| a["provider"] == "grok").unwrap();
            assert_eq!(grok["include_context"], false);
            call(&core, "set_provider_key", json!({ "provider": "grok", "key": "xai-test" })).await.unwrap();
            let st = call(&core, "list_providers", Value::Null).await.unwrap();
            assert!(st.as_array().unwrap().iter().any(|p| p["id"] == "grok" && p["connected"] == true));
        }
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
