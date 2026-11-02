//! Every operation the UI can ask for, by name. The desktop app forwards Tauri
//! `invoke` calls here and the server forwards `POST /api/call/<name>`, so both
//! hosts expose exactly the same API. Argument names are camelCase, matching
//! what the frontend has always sent to Tauri.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::secrets::CLAUDE_KEY;
use crate::{atera, claude, cursor, db, mail, ollama, openai, pocket, pomodoro, providers, transfer, unifi, Core};

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
    /// How often inboxes are checked for new mail.
    sync_seconds: u64,
}

/// Runs one named command with JSON arguments and returns its JSON result.
pub async fn call(core: &Arc<Core>, command: &str, a: Value) -> CmdResult<Value> {
    match command {
        // Commands that can run agents (directly, or through Pocket and voice).
        // Approving is only ever called from the app's own UI, so approving is the user's decision.
        "approve_tool" => ok(approve_tool(core, args::<Id>(a)?.id).await?),
        "run_agent" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: i64,
                input: String,
            }
            let A { agent_id, input } = args(a)?;
            ok(run_agent(core, agent_id, input, "app").await?)
        }
        "ask_provider" => {
            #[derive(Deserialize)]
            struct A {
                provider: String,
                input: String,
            }
            let A { provider, input } = args(a)?;
            ok(ask_provider(core, &provider, input, "app").await?)
        }
        "voice_command" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                audio: String,
                #[serde(default)]
                mime: String,
                #[serde(default)]
                agent_id: Option<i64>,
                #[serde(default)]
                provider: Option<String>,
            }
            let A { audio, mime, agent_id, provider } = args(a)?;
            ok(crate::voice::handle_audio(core, &audio, &mime, crate::voice::Fallback { agent_id, provider }).await?)
        }
        "pocket_refresh" => ok(pocket::refresh(core).await?),
        "pocket_set_key" => {
            let key = args::<Key>(a)?.key;
            core.secrets.set(crate::secrets::POCKET_KEY, &key)?;
            if !key.trim().is_empty() {
                pocket::mark_connected(core)?;
            }
            ok(pocket::refresh(core).await?)
        }
        "pocket_run" => {
            #[derive(Deserialize)]
            struct A {
                id: i64,
                #[serde(default)]
                edit: Option<pocket::Edit>,
            }
            let A { id, edit } = args(a)?;
            ok(pocket::run(core, id, edit).await?)
        }
        "import_connectors" => {
            #[derive(Deserialize)]
            struct A {
                connectors: Vec<transfer::ConnectorBundle>,
            }
            ok(transfer::import(core, args::<A>(a)?.connectors).await?)
        }
        _ => call_data(core, command, a).await,
    }
}

/// Commands that don't run agents, which is what Local AI's tools call (so a
/// tool can never start another agent run).
pub(crate) async fn call_data(core: &Arc<Core>, command: &str, a: Value) -> CmdResult<Value> {
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
                sync_seconds: mail::sync_seconds(&c),
            })
        }
        "set_mail_sync_seconds" => {
            #[derive(Deserialize)]
            struct A {
                seconds: u64,
            }
            ok(mail::set_sync_seconds(&conn(), args::<A>(a)?.seconds)?)
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
        "mail_sign_in_prompt" => ok(mail::device_prompt()),
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
        // Sends and deletes queued by mail::change_mail. These are only ever called
        // from the app's own UI, so approving one is the user's decision.
        "list_mail_actions" => ok(db::list_mail_actions(&conn()).map_err(err)?),
        "list_tool_approvals" => ok(db::list_tool_approvals(&conn()).map_err(err)?),
        "reject_tool" => {
            let ap = db::decide_tool_approval(&conn(), args::<Id>(a)?.id, "rejected").map_err(err)?.ok_or("That request was already approved or rejected.")?;
            if let Some(run) = ap.run_id {
                db::append_run_output(&conn(), run, &format!("\n\n✗ You rejected: {}.", ap.summary)).map_err(err)?;
            }
            core.changed("approvals-changed");
            core.changed("runs-changed");
            ok(ap)
        }
        "approve_mail_action" => ok(mail::run_approved(core, args::<Id>(a)?.id).await?),
        "reject_mail_action" => {
            let id = args::<Id>(a)?.id;
            db::decide_mail_action(&conn(), id, "rejected").map_err(err)?.ok_or("That mail action was already approved or rejected.")?;
            core.changed("mail-actions-changed");
            ok(())
        }

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
        "clear_chat" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: i64,
            }
            db::clear_chat(&conn(), args::<A>(a)?.agent_id).map_err(err)?;
            core.changed("agents-changed");
            ok(())
        }
        "list_memories" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: i64,
            }
            ok(db::list_memories(&conn(), args::<A>(a)?.agent_id).map_err(err)?)
        }
        "delete_memory" => {
            db::delete_memory(&conn(), args::<Id>(a)?.id).map_err(err)?;
            core.changed("memories-changed");
            ok(())
        }
        "clear_memories" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                agent_id: i64,
            }
            db::clear_memories(&conn(), args::<A>(a)?.agent_id).map_err(err)?;
            core.changed("memories-changed");
            ok(())
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
        "list_providers" => ok(providers::statuses(&core.secrets).await?),
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

        // ---------- Push-to-talk ----------
        "voice_status" => ok(crate::whisper::status().await),

        // ---------- Pocket recorder ----------
        "pocket_status" => ok(core.pocket.snapshot(core)?),
        "pocket_set_webhook_secret" => {
            let key = args::<Key>(a)?.key;
            core.secrets.set(crate::secrets::POCKET_WEBHOOK_SECRET, &key)?;
            if !key.trim().is_empty() {
                pocket::mark_connected(core)?;
            }
            core.changed("pocket-changed");
            ok(core.pocket.snapshot(core)?)
        }
        "pocket_set_options" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                wake: String,
                auto: pocket::Auto,
                ai: bool,
                #[serde(default)]
                ai_model: String,
            }
            let A { wake, auto, ai, ai_model } = args(a)?;
            pocket::set_options(core, &wake, auto, ai, &ai_model)?;
            ok(core.pocket.snapshot(core)?)
        }
        "pocket_set_triggers" => {
            #[derive(Deserialize)]
            struct A {
                triggers: Vec<pocket::Trigger>,
            }
            pocket::set_triggers(core, args::<A>(a)?.triggers)?;
            ok(core.pocket.snapshot(core)?)
        }
        "pocket_dismiss" => ok(pocket::dismiss(core, args::<Id>(a)?.id)?),

        // ---------- Connectors moving between Command Centers ----------
        // There is deliberately no export command: keys never leave over the API.
        "connector_summary" => ok(transfer::summary(core)?),

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

/// "chatgpt: ..." from the dashboard or a voice command: uses the provider's
/// first agent, making a plain one the first time.
pub(crate) async fn ask_provider(core: &Arc<Core>, provider: &str, input: String, source: &str) -> CmdResult<db::AgentRun> {
    let p = providers::find(provider).ok_or("Unknown AI provider.")?;
    if let Some(why) = p.unavailable {
        return Err(why.into());
    }
    let agent_id = {
        let c = core.db.0.lock().unwrap();
        match db::list_agents(&c).map_err(err)?.into_iter().find(|a| a.provider == p.id) {
            Some(a) => a.id,
            None if p.background => {
                return Err(format!("Make a {} agent under Agents first, so it knows which repository to work on.", p.name))
            }
            None => db::save_agent(&c, providers::quick_agent(p)).map_err(err)?.id,
        }
    };
    run_agent(core, agent_id, input, source).await
}

/// How much of a chat each run sends back, so long chats cost about the same
/// per message: the last 20 messages (10 exchanges), and at most ~6k tokens.
const CHAT_MAX_EXCHANGES: i64 = 10;
const CHAT_MAX_CHARS: usize = 24_000;
/// A local model sees only `ollama::num_ctx()` tokens in all (8k by default),
/// and its thinking and answer need room too, so its chat gets ~3.5k tokens.
const LOCAL_CHAT_MAX_CHARS: usize = 14_000;

/// Whether an agent keeps facts about the user across chats. Local AI only for
/// now: picking facts out runs on the server's own model, so it costs nothing.
fn learns(agent: &db::Agent) -> bool {
    agent.memory && agent.provider == "local"
}

/// Drops the oldest exchanges until the rest fit in `max_chars`.
fn fit_history(mut history: Vec<(String, String)>, max_chars: usize) -> Vec<(String, String)> {
    let size = |h: &[(String, String)]| h.iter().map(|(q, a)| q.len() + a.len()).sum::<usize>();
    while !history.is_empty() && size(&history) > max_chars {
        history.remove(0);
    }
    history
}

/// The agent's own instructions plus a short note on who it is, whether it
/// remembers the chat, and which other agents exist. Without it a model asked
/// about "your memory" or "my other bots" says it has none.
fn agent_system(agent: &db::Agent, others: &[db::Agent], facts: &[String]) -> String {
    let memory = if learns(agent) {
        "The earlier messages of this chat are included, so you remember them; older ones drop off once the chat gets long. \
         You also keep a long-term memory of facts about the user that lasts across chats (listed below when there are any); \
         new facts from this chat are saved to it automatically after you answer, and the user can see and delete them under Agents."
    } else if agent.memory {
        "The earlier messages of this chat are included, so you remember them; older ones drop off once the chat gets long."
    } else {
        "Memory is off for you, so each message starts fresh."
    };
    let mut note = format!(
        "You are \"{}\", one of the user's AI agents in their Command Center, a dashboard where they talk to each agent by name (typed, or by voice through their Pocket recorder). {memory}",
        agent.name.trim()
    );
    note.push_str(" You can't see the user's chats, bots or history in other apps, such as the Grok or ChatGPT apps; say so if asked.");
    let names: Vec<String> = others
        .iter()
        .filter(|a| a.id != agent.id)
        .map(|a| format!("{} ({})", a.name.trim(), providers::find(&a.provider).map_or(a.provider.as_str(), |p| p.name)))
        .collect();
    if !names.is_empty() {
        note.push_str(&format!(
            " The other agents are: {}. {}",
            names.join(", "),
            if agent.provider == "local" && tools_on() {
                "You can read what they were asked and answered recently with a tool, but you can't message them yet; the user talks to each one by name."
            } else {
                "You can't see their chats or message them; the user talks to each one by name."
            }
        ));
    }
    if !facts.is_empty() {
        note.push_str("\n\nWhat you remember about the user from earlier chats (use it when it helps; don't recite it):");
        for f in facts {
            note.push_str("\n- ");
            note.push_str(f);
        }
    }
    match agent.system_prompt.trim() {
        "" => note,
        own => format!("{own}\n\n{note}"),
    }
}

/// `source` is where the request came from ("app" or "pocket"), shown in the chat.
pub(crate) async fn run_agent(core: &Arc<Core>, agent_id: i64, input: String, source: &str) -> CmdResult<db::AgentRun> {
    let agent = db::get_agent(&core.db.0.lock().unwrap(), agent_id).map_err(err)?.ok_or("That agent no longer exists.")?;
    let provider = providers::find(&agent.provider).ok_or("This agent's AI provider isn't supported.")?;
    if let Some(why) = provider.unavailable {
        return Err(why.into());
    }
    let api_key = match provider.key {
        Some(name) => core
            .secrets
            .get(name)?
            .ok_or_else(|| format!("Add your {} key in Settings > Connections before running this agent.", provider.name))?,
        None if provider.local => String::new(),
        None => return Err("This provider doesn't take a key.".into()),
    };
    if provider.background && input.trim().is_empty() && agent.system_prompt.trim().is_empty() {
        return Err(format!("Tell {} what to do first.", provider.name));
    }

    // Hold the lock only for database work, never across the network call.
    let (run_id, context, history, system) = {
        let conn = core.db.0.lock().unwrap();
        let input_label = if input.trim().is_empty() { "(no extra input)" } else { input.trim() };
        let run_id = db::start_run(&conn, agent_id, input_label, source).map_err(err)?;
        let context = if agent.include_context { day_context(&conn)? } else { String::new() };
        let budget = if provider.local { LOCAL_CHAT_MAX_CHARS } else { CHAT_MAX_CHARS };
        let history = if agent.memory && !provider.background {
            let rows = db::chat_history_runs(&conn, agent_id, run_id, CHAT_MAX_EXCHANGES).map_err(err)?;
            let rows = if provider.local { rows.into_iter().filter(|(q, a, used)| trustworthy(q, a, used)).collect() } else { rows };
            fit_history(rows.into_iter().map(|(q, a, _)| (q, a)).collect(), budget)
        } else {
            vec![]
        };
        let facts: Vec<String> = if learns(&agent) {
            db::list_memories(&conn, agent_id).map_err(err)?.into_iter().map(|m| m.fact).collect()
        } else {
            vec![]
        };
        let system = agent_system(&agent, &db::list_agents(&conn).map_err(err)?, &facts);
        (run_id, context, history, system)
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

    let mut unreliable = false;
    let result = match provider.id {
        "chatgpt" => openai::chatgpt(&api_key, &agent.model, &system, &history, &user).await,
        "grok" => openai::grok(&api_key, &agent.model, &system, &history, &user).await,
        "local" if tools_on() => {
            let recent = history.iter().rev().take(2).map(|(q, a)| format!("{q} {a}")).collect::<Vec<_>>().join(" ");
            let picked = crate::tools::pick(input.trim(), &recent);
            let system = format!("{system}\n\n{}", crate::tools::system_note(chrono::Local::now()));
            let ctx = crate::tools::Ctx { run_id: Some(run_id), agent_id: Some(agent_id) };
            let asked = Asked { ctx, input: input.trim(), about_data: crate::tools::about_data(input.trim(), "") };
            match local_with_tools(core, &asked, &agent.model, &system, &history, &user, &picked).await {
                Ok((c, used)) => {
                    db::set_run_tools(&core.db.0.lock().unwrap(), run_id, &used).map_err(err)?;
                    // Don't learn from an answer that went wrong; it may say things that aren't so.
                    unreliable = used.iter().any(|u| !u.ok) || c.text.ends_with(NOTHING_CHANGED);
                    Ok(c)
                }
                Err(e) => Err(e),
            }
        }
        "local" => ollama::complete(&agent.model, &system, &history, &user).await,
        _ => claude::complete(&api_key, &agent.model, &system, &history, &user).await,
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
    // Actions Local AI may take without a tap (asking Grok) run now its reply is in.
    let run = if provider.id == "local" { run_auto_approved(core, run).await? } else { run };
    if learns(&agent) && run.status == "done" && !input.trim().is_empty() && !unreliable {
        let (core, agent, said, reply) = (core.clone(), agent.clone(), input.trim().to_string(), run.output.clone());
        tokio::spawn(async move {
            if let Err(e) = learn(&core, &agent, &said, &reply).await {
                eprintln!("long-term memory for {}: {e}", agent.name);
            }
        });
    }
    Ok(run)
}

/// Carries out the requests from one Local AI run that need no tap, and
/// returns the run with their outcome added.
async fn run_auto_approved(core: &Arc<Core>, run: db::AgentRun) -> CmdResult<db::AgentRun> {
    let auto: Vec<i64> = db::list_tool_approvals(&core.db.0.lock().unwrap())
        .map_err(err)?
        .into_iter()
        .filter(|a| a.run_id == Some(run.id) && a.status == "pending" && auto_approved(a))
        .map(|a| a.id)
        .collect();
    if auto.is_empty() {
        return Ok(run);
    }
    for id in auto {
        if let Err(e) = Box::pin(approve_tool(core, id)).await {
            eprintln!("auto-approved request {id}: {e}");
        }
    }
    let conn = core.db.0.lock().unwrap();
    Ok(db::get_run(&conn, run.id).map_err(err)?.unwrap_or(run))
}

/// Whether a stored request may run without a tap. Checked again here, not
/// just trusted from the stored flag, so only the AIs allowed today qualify.
fn auto_approved(a: &db::ToolApproval) -> bool {
    let args: Value = serde_json::from_str(&a.args).unwrap_or_default();
    a.tool == "ask_ai" && args["auto"].as_bool() == Some(true) && args["ai"].as_str().is_some_and(crate::tools::runs_without_asking)
}

/// Runs an action Local AI asked for, after the user approved it. Only the
/// "approve_tool" command (the app's own Approve button) calls this.
pub(crate) async fn approve_tool(core: &Arc<Core>, id: i64) -> CmdResult<db::ToolApproval> {
    {
        let conn = core.db.0.lock().unwrap();
        let pending = db::list_tool_approvals(&conn).map_err(err)?.into_iter().find(|a| a.id == id);
        let run = pending.and_then(|a| a.run_id).and_then(|r| db::get_run(&conn, r).ok().flatten());
        if run.is_some_and(|r| r.status == "running") {
            return Err("Local AI is still answering. Approve it once the answer is in.".into());
        }
    }
    let ap = db::decide_tool_approval(&core.db.0.lock().unwrap(), id, "approved").map_err(err)?.ok_or("That request was already approved or rejected.")?;
    core.changed("approvals-changed");
    let a: Value = serde_json::from_str(&ap.args).map_err(|e| format!("The stored request is not valid JSON: {e}"))?;
    let clip = |t: &str| if t.chars().count() <= 600 { t.to_string() } else { format!("{}…", t.chars().take(600).collect::<String>()) };
    let result: CmdResult<String> = match ap.tool.as_str() {
        "delete_todo" | "delete_reminder" | "delete_project" => {
            call_data(core, &ap.tool, serde_json::json!({ "id": a["id"] })).await.map(|_| "Deleted.".to_string())
        }
        "ask_ai" => {
            let ai = a["ai"].as_str().unwrap_or_default().to_string();
            let q = a["question"].as_str().unwrap_or_default().to_string();
            match Box::pin(ask_provider(core, &ai, q, "app")).await {
                Ok(r) if r.status == "done" => Ok(format!("{} answered: {}", r.agent_name, clip(&r.output))),
                Ok(r) => Err(clip(&r.output)),
                Err(e) => Err(e),
            }
        }
        "cursor_task" => {
            let agent_id = a["agent_id"].as_i64().ok_or("The stored request has no agent.")?;
            let what = a["instructions"].as_str().unwrap_or_default().to_string();
            match Box::pin(run_agent(core, agent_id, what, "app")).await {
                Ok(r) if r.status == "error" => Err(clip(&r.output)),
                Ok(r) => Ok(format!("Cursor started{}.", if r.link.is_empty() { String::new() } else { format!(": {}", r.link) })),
                Err(e) => Err(e),
            }
        }
        other => Err(format!("{other} can't be approved here.")),
    };
    let (ok_, text) = match &result {
        Ok(t) => (true, t.clone()),
        Err(e) => (false, e.clone()),
    };
    {
        let conn = core.db.0.lock().unwrap();
        db::finish_tool_approval(&conn, id, ok_, &text).map_err(err)?;
        if let Some(run) = ap.run_id {
            let line = if ok_ { format!("\n\n✓ Approved: {}. {text}", ap.summary) } else { format!("\n\n✗ Approved but failed: {}. {text}", ap.summary) };
            db::append_run_output(&conn, run, &line).map_err(err)?;
        }
    }
    core.changed("approvals-changed");
    core.changed("runs-changed");
    let conn = core.db.0.lock().unwrap();
    Ok(db::list_tool_approvals(&conn).map_err(err)?.into_iter().find(|x| x.id == id).unwrap_or(ap))
}

/// Local AI uses tools unless `CC_LOCAL_TOOLS=0` turns them off.
fn tools_on() -> bool {
    std::env::var("CC_LOCAL_TOOLS").ok().is_none_or(|v| v.trim() != "0")
}

/// How many times one message may go back and forth with tools before the
/// model has to answer with what it has.
const TOOL_ROUNDS: usize = 5;
/// At most this many tool calls in one round, so a confused model can't fan out.
const TOOL_CALLS_PER_ROUND: usize = 6;

/// A Local AI answer that may call tools first. Returns the answer and the
/// tools it used, in order.
async fn local_with_tools(
    core: &Arc<Core>,
    asked: &Asked<'_>,
    model: &str,
    system: &str,
    history: &claude::History,
    user: &str,
    picked: &[&'static crate::tools::Tool],
) -> CmdResult<(claude::Completion, Vec<crate::tools::Used>)> {
    let model = ollama::pick_model(model).await?;
    let step = |messages: Vec<Value>, tools: Vec<Value>, think: bool| {
        let model = model.clone();
        async move { ollama::chat_step(&model, &messages, &tools, think).await }
    };
    tool_loop(core, asked, system, history, user, picked, step).await
}

/// The back-and-forth behind `local_with_tools`; `step` asks the model once.
async fn tool_loop<F, Fut>(
    core: &Arc<Core>,
    asked: &Asked<'_>,
    system: &str,
    history: &claude::History,
    user: &str,
    picked: &[&'static crate::tools::Tool],
    step: F,
) -> CmdResult<(claude::Completion, Vec<crate::tools::Used>)>
where
    // (messages, tools, think): `think` is on for the retry after a check.
    F: Fn(Vec<Value>, Vec<Value>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<(Value, claude::Completion), String>>,
{
    let schema = crate::tools::schema(picked);
    let mut messages = vec![serde_json::json!({ "role": "system", "content": system })];
    messages.extend(claude::messages(history, user));
    let mut used: Vec<crate::tools::Used> = vec![];
    let (mut input_tokens, mut output_tokens) = (0, 0);
    let mut nudged = false;
    for round in 0..=TOOL_ROUNDS {
        // The last round offers no tools, so the model has to answer.
        let offer: Vec<Value> = if round == TOOL_ROUNDS { vec![] } else { schema.clone() };
        let no_tools = offer.is_empty();
        let (message, c) = step(messages.clone(), offer, nudged).await?;
        input_tokens += c.input_tokens;
        output_tokens += c.output_tokens;
        let calls = ollama::tool_calls(&message);
        if calls.is_empty() || no_tools {
            // A small model sometimes answers "done" without calling anything, or
            // from an old answer in the chat. Send it back once.
            let wants = crate::tools::wants_change(asked.input);
            let nudge = if wants && !crate::tools::changed(&used) {
                Some(NUDGE_CHANGE)
            } else if used.is_empty() && asked.about_data {
                Some(NUDGE_LOOK)
            } else {
                None
            };
            if let (Some(n), false, false) = (nudge, nudged, no_tools) {
                // The wrong answer is left out, so the model doesn't read its own
                // "done" back; the request is repeated, and it may think this time.
                nudged = true;
                messages.push(serde_json::json!({ "role": "user", "content": format!("{n}\n\nThe request: {}", asked.input) }));
                continue;
            }
            let mut text = if c.text.is_empty() && !used.is_empty() { "Done.".to_string() } else { c.text };
            if wants && !crate::tools::changed(&used) {
                // Even an answer that doesn't claim anything gets this, so a failed
                // change is never silent.
                text.push_str(NOTHING_CHANGED);
            }
            return Ok((claude::Completion { text, input_tokens, output_tokens, ..c }, used));
        }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": message["content"].as_str().unwrap_or_default(),
            "tool_calls": message["tool_calls"].clone(),
        }));
        for (i, (name, args)) in calls.into_iter().enumerate() {
            let (result, u) = if i >= TOOL_CALLS_PER_ROUND {
                ("Error: too many tool calls at once; use fewer.".to_string(), None)
            } else if !picked.iter().any(|t| t.name == name) {
                // Only tools offered for this message run, even if the model knows other names.
                (format!("Error: {name} isn't available for this message."), None)
            } else {
                let args = drop_made_up_due(&name, args, asked.input);
                let (r, u) = crate::tools::run(core, asked.ctx, &name, &args).await;
                (r, Some(u))
            };
            used.extend(u);
            messages.push(serde_json::json!({ "role": "tool", "tool_name": name, "content": result }));
        }
    }
    Err("Local AI kept calling tools without answering.".into())
}

/// Whether an earlier Local AI exchange is safe to show the model again. One
/// that went wrong (a failed tool, a change asked for that no tool made) teaches
/// a small model to repeat the mistake, so it is left out of the chat it sees.
fn trustworthy(asked: &str, answer: &str, used: &[crate::tools::Used]) -> bool {
    !answer.contains(NOTHING_CHANGED.trim())
        && used.iter().all(|u| u.ok)
        && !(crate::tools::wants_change(asked) && !crate::tools::changed(used))
}

/// A small model sometimes gives a new todo a due date nobody asked for. Keep
/// one only when the message says when.
fn drop_made_up_due(tool: &str, mut args: Value, said: &str) -> Value {
    if tool == "add_todo" && !crate::tools::mentions_time(said) {
        if let Some(o) = args.as_object_mut() {
            o.remove("due");
        }
    }
    args
}

/// What the user asked, apart from the day's context sent with it.
pub(crate) struct Asked<'a> {
    /// The run the tools report into.
    pub ctx: crate::tools::Ctx,
    pub input: &'a str,
    /// The message (or the end of the chat) is about the user's data, so the
    /// answer should come from a tool, not from memory of earlier answers.
    pub about_data: bool,
}

const NUDGE_CHANGE: &str = "[Command Center] No tool was called, so nothing has changed. Call the tool that does this now \
(name the item by its title if you don't have its id). Earlier replies in this chat that say \"done\" may be wrong.";
const NUDGE_LOOK: &str = "[Command Center] Your earlier answers may be out of date. Call a tool to get the current data, then answer from it.";
/// Added to an answer that claims a change no tool made.
const NOTHING_CHANGED: &str = "\n\n(Command Center: nothing was changed. Local AI didn't complete a tool for this, so please check or ask again.)";

const LEARN_SYSTEM: &str = "You keep an AI assistant's long-term memory: a short list of lasting facts about its user. \
You get the current list (numbered) and the newest exchange. Answer with JSON: \
\"add\" = new lasting facts the exchange reveals, and \"forget\" = numbers of listed facts that are now wrong, outdated, or the user asked to forget. \
Worth keeping: the user's name and how to address them, people and pets in their life, their job, projects, tools and devices, \
preferences and dislikes, plans and dates, standing instructions, and anything they say to remember. \
Not worth keeping: one-off questions, small talk, facts about the world, what the assistant said or did, anything already listed, \
and the current state of their todos, reminders, alerts, network sites, devices or inbox (the app already tracks those, and they change). \
Write each fact as one short sentence about the user in third person, such as \"Prefers metric units.\" \
Most exchanges have nothing new: then both lists are empty.";

fn learn_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "add": { "type": "array", "items": { "type": "string" } },
            "forget": { "type": "array", "items": { "type": "integer" } }
        },
        "required": ["add", "forget"]
    })
}

fn learn_prompt(facts: &[db::AgentMemory], said: &str, reply: &str) -> String {
    let clip = |t: &str, n: usize| if t.chars().count() <= n { t.to_string() } else { format!("{}…", t.chars().take(n).collect::<String>()) };
    let listed = if facts.is_empty() {
        "(none yet)".to_string()
    } else {
        facts.iter().enumerate().map(|(i, f)| format!("{}. {}", i + 1, f.fact)).collect::<Vec<_>>().join("\n")
    };
    format!("Current facts:\n{listed}\n\nUser said:\n{}\n\nAssistant answered:\n{}", clip(said, 4000), clip(reply, 3000))
}

/// Which remembered facts to drop and add, from the model's answer.
fn learned(v: &Value, facts: &[db::AgentMemory]) -> (Vec<i64>, Vec<String>) {
    let forget = v["forget"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_u64())
        .filter_map(|n| facts.get((n as usize).checked_sub(1)?).map(|f| f.id))
        .collect();
    let add = v["add"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).take(5).collect();
    (forget, add)
}

/// Picks lasting facts out of one finished exchange, using the agent's own
/// local model, and saves them to its long-term memory.
async fn learn(core: &Arc<Core>, agent: &db::Agent, said: &str, reply: &str) -> Result<(), String> {
    let facts = db::list_memories(&core.db.0.lock().unwrap(), agent.id).map_err(err)?;
    let v = ollama::complete_json_quick(
        &agent.model,
        LEARN_SYSTEM,
        &learn_prompt(&facts, said, reply),
        learn_schema(),
        std::time::Duration::from_secs(180),
    )
    .await?;
    let (forget, add) = learned(&v, &facts);
    if forget.is_empty() && add.is_empty() {
        return Ok(());
    }
    {
        let conn = core.db.0.lock().unwrap();
        for id in forget {
            db::delete_memory(&conn, id).map_err(err)?;
        }
        db::add_memories(&conn, agent.id, &add).map_err(err)?;
    }
    core.changed("memories-changed");
    Ok(())
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

    #[test]
    fn agents_know_their_name_memory_and_siblings() {
        let agent = |id: i64, name: &str, provider: &str, prompt: &str, memory: bool| db::Agent {
            id,
            name: name.into(),
            description: String::new(),
            system_prompt: prompt.into(),
            model: String::new(),
            created_at: String::new(),
            provider: provider.into(),
            repo: String::new(),
            include_context: false,
            memory,
            chat_since: None,
        };
        let max = agent(1, "Max", "grok", "You are a sarcastic researcher.", true);
        let all = [max.clone(), agent(2, "Daily Planner", "claude", "", false)];
        let s = agent_system(&max, &all, &[]);
        assert!(s.starts_with("You are a sarcastic researcher.\n\n"));
        assert!(s.contains("\"Max\"") && s.contains("you remember them") && s.contains("Grok or ChatGPT apps"));
        assert!(s.contains("Daily Planner (") && !s.contains("Max ("));
        let alone = agent_system(&agent(3, "Solo", "local", "", false), &[], &[]);
        assert!(alone.starts_with("You are \"Solo\"") && alone.contains("Memory is off") && !alone.contains("other agents"));
        let local = agent(4, "Local AI", "local", "", true);
        let s = agent_system(&local, &[], &["Has a dog named Rex.".into()]);
        assert!(s.contains("long-term memory") && s.ends_with("earlier chats (use it when it helps; don't recite it):\n- Has a dog named Rex."));
        assert!(!agent_system(&max, &all, &[]).contains("long-term memory"));
    }

    #[test]
    fn learned_facts_are_read_safely() {
        let m = |id: i64, fact: &str| db::AgentMemory { id, agent_id: 1, fact: fact.into(), created_at: String::new() };
        let facts = [m(7, "Lives in Denver."), m(9, "Drinks coffee.")];
        let v = json!({ "add": ["Moved to Boston.", "  ", 3], "forget": [1, 0, 5, "2"] });
        assert_eq!(learned(&v, &facts), (vec![7], vec!["Moved to Boston.".to_string()]));
        assert_eq!(learned(&json!({}), &facts), (vec![], vec![]));
        let p = learn_prompt(&facts, "hi", &"x".repeat(5000));
        assert!(p.contains("1. Lives in Denver.\n2. Drinks coffee.") && p.ends_with("…"));
        assert!(learn_prompt(&[], "hi", "yo").contains("(none yet)"));
    }

    #[tokio::test]
    async fn memory_commands_list_delete_and_clear() {
        let core = core();
        let local = call(&core, "save_agent", json!({ "agent": { "name": "Local AI", "model": "", "provider": "local", "system_prompt": "Be brief." } })).await.unwrap();
        let id = local["id"].as_i64().unwrap();
        assert_eq!(local["memory"], true);
        db::add_memories(&core.db.0.lock().unwrap(), id, &["One.".into(), "Two.".into()]).unwrap();
        let list = call(&core, "list_memories", json!({ "agentId": id })).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
        call(&core, "delete_memory", json!({ "id": list[0]["id"] })).await.unwrap();
        assert_eq!(call(&core, "list_memories", json!({ "agentId": id })).await.unwrap()[0]["fact"], "Two.");
        call(&core, "clear_memories", json!({ "agentId": id })).await.unwrap();
        assert_eq!(call(&core, "list_memories", json!({ "agentId": id })).await.unwrap(), json!([]));
    }

    #[tokio::test]
    async fn local_ai_uses_tools_then_answers() {
        let core = core();
        let picked = crate::tools::pick("add a todo to call Dell", "");
        let seen = std::sync::Mutex::new(vec![]);
        let step = |messages: Vec<Value>, tools: Vec<Value>, _think: bool| {
            seen.lock().unwrap().push((messages, tools.len()));
            let round = seen.lock().unwrap().len();
            let c = |text: &str| claude::Completion { text: text.into(), stop_reason: "end_turn".into(), model: "qwen3:8b".into(), input_tokens: 10, output_tokens: 2 };
            let out = match round {
                // A tool it wasn't offered is refused; a good one runs.
                1 => (json!({ "role": "assistant", "content": "", "tool_calls": [
                    { "function": { "name": "atera_alerts", "arguments": {} } },
                    { "function": { "name": "add_todo", "arguments": { "title": "Call Dell", "priority": "high" } } }
                ]}), c("")),
                _ => (json!({ "role": "assistant", "content": "Added it." }), c("Added it.")),
            };
            async move { Ok(out) }
        };
        let (answer, used) = tool_loop(&core, &Asked { ctx: Default::default(), input: "add a todo to call Dell", about_data: true }, "sys", &[], "add a todo to call Dell", &picked, step).await.unwrap();
        assert_eq!((answer.text.as_str(), answer.input_tokens), ("Added it.", 20));
        assert_eq!(used.len(), 1);
        assert_eq!(used[0].label, "Added todo “Call Dell”");
        let todos = db::list_todos(&core.db.0.lock().unwrap()).unwrap();
        assert_eq!((todos[0].title.as_str(), todos[0].priority), ("Call Dell", 1));
        let seen = seen.lock().unwrap();
        let tool_msgs: Vec<&Value> = seen[1].0.iter().filter(|m| m["role"] == "tool").collect();
        assert!(tool_msgs[0]["content"].as_str().unwrap().contains("isn't available"));
        assert!(tool_msgs[1]["content"].as_str().unwrap().starts_with("Added todo #"));
        assert!(seen[0].1 > 0);
    }

    #[tokio::test]
    async fn ask_first_tools_wait_for_approval() {
        let core = core();
        let todo = call(&core, "add_todo", json!({ "todo": { "title": "Old task" } })).await.unwrap();
        let id = todo["id"].as_i64().unwrap();
        let (agent, run) = {
            let c = core.db.0.lock().unwrap();
            let agent = db::save_agent(&c, serde_json::from_value(json!({ "name": "Local AI", "model": "", "provider": "local", "system_prompt": "x" })).unwrap()).unwrap();
            let run = db::start_run(&c, agent.id, "delete old task", "app").unwrap();
            db::finish_run(&c, run, "done", "It's waiting for you.", "", 0, 0).unwrap();
            (agent, run)
        };
        let ctx = crate::tools::Ctx { run_id: Some(run), agent_id: Some(agent.id) };
        let (text, used) = crate::tools::run(&core, ctx, "delete_todo", &json!({ "id": id })).await;
        assert!(text.contains("NOT happened") && used.waiting, "{text}");
        // Nothing deleted until approved.
        assert_eq!(call(&core, "list_todos", Value::Null).await.unwrap().as_array().unwrap().len(), 1);
        let pending = call(&core, "list_tool_approvals", Value::Null).await.unwrap();
        assert_eq!((pending[0]["status"].as_str(), pending[0]["summary"].as_str()), (Some("pending"), Some(format!("Delete todo #{id} “Old task”").as_str())));
        let done = call(&core, "approve_tool", json!({ "id": pending[0]["id"] })).await.unwrap();
        assert_eq!(done["status"], "done");
        assert_eq!(call(&core, "list_todos", Value::Null).await.unwrap(), json!([]));
        assert!(call(&core, "approve_tool", json!({ "id": pending[0]["id"] })).await.is_err(), "runs once");
        let out = db::get_run(&core.db.0.lock().unwrap(), run).unwrap().unwrap().output;
        assert!(out.ends_with(&format!("✓ Approved: Delete todo #{id} “Old task”. Deleted.")), "{out}");

        // Rejected requests never run.
        let r = call(&core, "add_reminder", json!({ "reminder": { "title": "Ping", "remind_at": "2030-01-01T09:00:00Z" } })).await.unwrap();
        crate::tools::run(&core, ctx, "delete_reminder", &json!({ "id": r["id"] })).await;
        let ap = call(&core, "list_tool_approvals", Value::Null).await.unwrap()[0]["id"].clone();
        call(&core, "reject_tool", json!({ "id": ap })).await.unwrap();
        assert!(call(&core, "approve_tool", json!({ "id": ap })).await.is_err());
        assert_eq!(call(&core, "list_reminders", Value::Null).await.unwrap().as_array().unwrap().len(), 1);

        // Bad ids and AIs without keys are refused up front.
        let (text, used) = crate::tools::run(&core, ctx, "delete_todo", &json!({ "id": 999 })).await;
        assert!(text.contains("no todo #999") && text.contains("list_todos") && !used.ok);
        if std::env::var("XAI_API_KEY").is_err() {
            let (text, _) = crate::tools::run(&core, ctx, "ask_ai", &json!({ "ai": "grok", "question": "hi" })).await;
            assert!(text.contains("no key"), "{text}");
        }
        let (text, _) = crate::tools::run(&core, ctx, "send_email", &json!({ "to": "a@b.com", "subject": "Hi", "body": "x" })).await;
        assert!(text.contains("No inbox"), "{text}");
    }

    #[tokio::test]
    async fn grok_is_asked_without_a_tap_but_chatgpt_waits() {
        let core = core();
        core.secrets.set(crate::secrets::XAI_KEY, "test").unwrap();
        core.secrets.set(crate::secrets::OPENAI_KEY, "test").unwrap();
        let ctx = crate::tools::Ctx::default();
        let (text, used) = crate::tools::run(&core, ctx, "ask_ai", &json!({ "ai": "grok", "question": "What's new?" })).await;
        assert!(text.starts_with("Done: Ask Grok") && used.ok && !used.waiting, "{text}");
        let (text, used) = crate::tools::run(&core, ctx, "ask_ai", &json!({ "ai": "chatgpt", "question": "What's new?" })).await;
        assert!(text.contains("approval") && used.waiting, "{text}");
        let list = db::list_tool_approvals(&core.db.0.lock().unwrap()).unwrap();
        let auto: Vec<bool> = list.iter().map(auto_approved).collect();
        assert_eq!(auto, vec![false, true], "newest first: ChatGPT waits, Grok runs");
        // A stored flag alone isn't enough; only Grok qualifies.
        let mut forged = list[0].clone();
        forged.args = json!({ "ai": "chatgpt", "question": "x", "auto": true }).to_string();
        assert!(!auto_approved(&forged));
    }

    /// A stand-in model that answers from a script, one reply per call.
    fn scripted(replies: Vec<Value>) -> (std::sync::Mutex<Vec<Vec<Value>>>, std::sync::Mutex<std::collections::VecDeque<Value>>) {
        (std::sync::Mutex::new(vec![]), std::sync::Mutex::new(replies.into()))
    }

    #[tokio::test]
    async fn a_claimed_change_without_a_tool_is_sent_back_then_flagged() {
        let core = core();
        call(&core, "add_todo", json!({ "todo": { "title": "Test Local AI tools" } })).await.unwrap();
        let input = "mark the test Local AI tools todo done";
        let picked = crate::tools::pick(input, "");
        let asked = Asked { ctx: Default::default(), input, about_data: true };
        let c = |text: &str| claude::Completion { text: text.into(), stop_reason: "end_turn".into(), model: String::new(), input_tokens: 1, output_tokens: 1 };

        // Claims it twice without a tool: sent back once, then flagged.
        let (seen, script) = scripted(vec![json!({ "content": "Todo marked as done." }), json!({ "content": "Done, it's marked as done." })]);
        let step = |m: Vec<Value>, _: Vec<Value>, _: bool| {
            seen.lock().unwrap().push(m);
            let msg = script.lock().unwrap().pop_front().unwrap();
            let comp = c(msg["content"].as_str().unwrap());
            async move { Ok((msg, comp)) }
        };
        let (answer, used) = tool_loop(&core, &asked, "sys", &[], input, &picked, step).await.unwrap();
        assert!(used.is_empty());
        assert!(answer.text.ends_with(NOTHING_CHANGED), "{}", answer.text);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            let last = seen[1].last().unwrap();
            assert!(last["content"].as_str().unwrap().starts_with(NUDGE_CHANGE) && last["content"].as_str().unwrap().ends_with(input));
            // The false "done" isn't fed back.
            assert!(!seen[1].iter().any(|m| m["content"] == "Todo marked as done."));
        }
        assert!(!db::list_todos(&core.db.0.lock().unwrap()).unwrap()[0].done);

        // Sent back, it calls the tool by title (a guessed id falls back to the title) and it's real.
        let (_seen, script) = scripted(vec![
            json!({ "content": "Todo marked as done." }),
            json!({ "content": "", "tool_calls": [{ "function": { "name": "update_todo", "arguments": { "id": 1234, "title": "test local ai tools", "done": true } } }] }),
            json!({ "content": "Marked “Test Local AI tools” done." }),
        ]);
        let step = |_: Vec<Value>, _: Vec<Value>, _: bool| {
            let msg = script.lock().unwrap().pop_front().unwrap();
            let comp = c(msg["content"].as_str().unwrap());
            async move { Ok((msg, comp)) }
        };
        let (answer, used) = tool_loop(&core, &asked, "sys", &[], input, &picked, step).await.unwrap();
        assert_eq!(answer.text, "Marked “Test Local AI tools” done.");
        assert!(used[0].ok, "{:?}", used);
        assert!(db::list_todos(&core.db.0.lock().unwrap()).unwrap()[0].done);
    }

    #[test]
    fn todos_only_get_a_due_date_when_one_was_said() {
        let a = json!({ "title": "approval test", "priority": "low", "due": "2026-11-01 23:06" });
        assert!(drop_made_up_due("add_todo", a.clone(), "Add a todo: approval test, low priority").get("due").is_none());
        assert!(drop_made_up_due("add_todo", a.clone(), "add a todo to call Dell by Friday").get("due").is_some());
        assert!(drop_made_up_due("add_todo", a.clone(), "todo: renew cert, due 11/15").get("due").is_some());
        assert!(drop_made_up_due("add_reminder", a, "remind me").get("due").is_some());
    }

    #[test]
    fn bad_exchanges_stay_out_of_the_chat() {
        let u = |tool: &str, ok: bool| crate::tools::Used { tool: tool.into(), label: String::new(), ok, waiting: false };
        assert!(trustworthy("hi", "Hello.", &[]));
        assert!(trustworthy("add a todo: buy milk", "Added.", &[u("add_todo", true)]));
        assert!(!trustworthy("mark the test todo done", "Todo marked as done.", &[]), "old unflagged false claim");
        assert!(!trustworthy("what's due?", &format!("Nothing.{NOTHING_CHANGED}"), &[]));
        assert!(!trustworthy("delete the todo", "Gone.", &[u("delete_todo", false)]));
        assert!(trustworthy("delete the todo", "Waiting for you.", &[u("delete_todo", true)]));
    }

    #[tokio::test]
    async fn a_data_question_answered_from_memory_is_sent_to_look() {
        let core = core();
        let input = "Which UniFi sites have problems?";
        let picked = crate::tools::pick(input, "");
        let asked = Asked { ctx: Default::default(), input, about_data: crate::tools::about_data(input, "") };
        let (seen, script) = scripted(vec![
            json!({ "content": "Vail Chapel is down." }),
            json!({ "content": "", "tool_calls": [{ "function": { "name": "unifi_sites", "arguments": { "problems_only": true } } }] }),
            json!({ "content": "UniFi isn't connected." }),
        ]);
        let step = |m: Vec<Value>, _: Vec<Value>, _: bool| {
            seen.lock().unwrap().push(m);
            let msg = script.lock().unwrap().pop_front().unwrap();
            let comp = claude::Completion { text: msg["content"].as_str().unwrap().into(), stop_reason: "end_turn".into(), model: String::new(), input_tokens: 0, output_tokens: 0 };
            async move { Ok((msg, comp)) }
        };
        let (answer, used) = tool_loop(&core, &asked, "sys", &[], input, &picked, step).await.unwrap();
        assert_eq!(answer.text, "UniFi isn't connected.");
        assert_eq!(used[0].tool, "unifi_sites");
        assert!(seen.lock().unwrap()[1].last().unwrap()["content"].as_str().unwrap().starts_with(NUDGE_LOOK));
    }

    #[tokio::test]
    async fn tool_loop_stops_offering_tools_after_five_rounds() {
        let core = core();
        let picked = crate::tools::pick("todos", "");
        let rounds = std::sync::Mutex::new(vec![]);
        let step = |_: Vec<Value>, tools: Vec<Value>, _: bool| {
            rounds.lock().unwrap().push(tools.len());
            let c = claude::Completion { text: String::new(), stop_reason: "end_turn".into(), model: String::new(), input_tokens: 0, output_tokens: 0 };
            let msg = if tools.is_empty() {
                json!({ "content": "Here you go." })
            } else {
                json!({ "content": "", "tool_calls": [{ "function": { "name": "list_todos", "arguments": {} } }] })
            };
            let c = claude::Completion { text: msg["content"].as_str().unwrap().into(), ..c };
            async move { Ok((msg, c)) }
        };
        let (answer, used) = tool_loop(&core, &Asked { ctx: Default::default(), input: "todos", about_data: true }, "sys", &[], "todos", &picked, step).await.unwrap();
        assert_eq!(answer.text, "Here you go.");
        assert_eq!(used.len(), TOOL_ROUNDS);
        let r = rounds.lock().unwrap();
        assert_eq!(r.len(), TOOL_ROUNDS + 1);
        assert_eq!(*r.last().unwrap(), 0);
    }

    #[tokio::test]
    async fn runs_keep_the_tools_they_used() {
        let core = core();
        let c = core.db.0.lock().unwrap();
        let agent = db::save_agent(&c, serde_json::from_value(json!({ "name": "Local AI", "model": "", "provider": "local", "system_prompt": "Be brief." })).unwrap()).unwrap();
        let run = db::start_run(&c, agent.id, "hi", "app").unwrap();
        assert!(db::get_run(&c, run).unwrap().unwrap().tools.is_empty());
        let used = vec![crate::tools::Used { tool: "add_todo".into(), label: "Added todo “X”".into(), ok: true, waiting: false }];
        db::set_run_tools(&c, run, &used).unwrap();
        assert_eq!(db::get_run(&c, run).unwrap().unwrap().tools, used);
    }

    #[test]
    fn long_chats_drop_their_oldest_messages() {
        let h = |q: &str, a: &str| (q.to_string(), a.to_string());
        let chat = vec![h("aaaa", "aaaa"), h("bb", "bb"), h("c", "c")];
        assert_eq!(fit_history(chat.clone(), 100), chat);
        assert_eq!(fit_history(chat.clone(), 6), vec![h("bb", "bb"), h("c", "c")]);
        assert!(fit_history(chat, 1).is_empty());
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
            assert_eq!(grok["memory"], true);
            assert_eq!(grok["chat_since"], Value::Null);
            call(&core, "clear_chat", json!({ "agentId": grok["id"] })).await.unwrap();
            let agents = call(&core, "list_agents", Value::Null).await.unwrap();
            assert!(agents.as_array().unwrap().iter().any(|a| a["provider"] == "grok" && a["chat_since"].is_string()));
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
