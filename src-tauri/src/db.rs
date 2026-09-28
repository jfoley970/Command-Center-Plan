//! Local SQLite storage for projects, todos, reminders, agents and agent runs.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS todos (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    priority    INTEGER NOT NULL DEFAULT 2,  -- 1 high, 2 normal, 3 low
    due_at      TEXT,                        -- RFC 3339, optional
    done        INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    done_at     TEXT
);
CREATE TABLE IF NOT EXISTS reminders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL,
    remind_at   TEXT NOT NULL,               -- RFC 3339
    repeat      TEXT NOT NULL DEFAULT 'none',-- none | daily | weekly
    fired       INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    color       TEXT NOT NULL DEFAULT '#4c8dff',
    archived    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS mail_accounts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    email        TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL DEFAULT '',
    client_id    TEXT NOT NULL,
    tenant_id    TEXT NOT NULL,
    summary      TEXT NOT NULL DEFAULT '',
    summary_at   TEXT,
    last_sync_at TEXT,
    last_error   TEXT,
    created_at   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS emails (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,
    subject     TEXT NOT NULL DEFAULT '',
    from_name   TEXT NOT NULL DEFAULT '',
    from_addr   TEXT NOT NULL DEFAULT '',
    received_at TEXT NOT NULL,
    preview     TEXT NOT NULL DEFAULT '',
    body        TEXT NOT NULL DEFAULT '',
    is_read     INTEGER NOT NULL DEFAULT 0,
    web_link    TEXT NOT NULL DEFAULT '',
    analyzed    INTEGER NOT NULL DEFAULT 0,  -- 1 once Claude has looked for tasks in it
    UNIQUE (account_id, remote_id)
);
CREATE TABLE IF NOT EXISTS suggestions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    email_id    INTEGER REFERENCES emails(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,               -- todo | reminder
    title       TEXT NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    due_at      TEXT,                        -- RFC 3339; the reminder time for reminders
    priority    INTEGER NOT NULL DEFAULT 2,
    project_id  INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    status      TEXT NOT NULL DEFAULT 'pending', -- pending | accepted | dismissed
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agents (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    system_prompt TEXT NOT NULL,
    model         TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_runs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id      INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    input         TEXT NOT NULL,
    output        TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL,             -- running | done | error | refused
    model         TEXT NOT NULL DEFAULT '',
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    started_at    TEXT NOT NULL,
    finished_at   TEXT
);
"#;

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        seed_default_agent(&conn)?;
        Ok(Db(Mutex::new(conn)))
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|n| n == column))
}

/// Adds columns introduced after the first release to existing databases.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    for table in ["todos", "reminders"] {
        if !has_column(conn, table, "project_id")? {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL"
            ))?;
        }
        if !has_column(conn, table, "email_id")? {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN email_id INTEGER REFERENCES emails(id) ON DELETE SET NULL"
            ))?;
        }
    }
    Ok(())
}

fn seed_default_agent(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))?;
    if count == 0 {
        conn.execute(
            "INSERT INTO agents (name, description, system_prompt, model, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "Daily Planner",
                "Turns your open todos and reminders into a plan for the day.",
                "You are James's daily planning assistant. Given his open todos and upcoming reminders, \
                 produce a short, prioritized plan for today: the three most important items first, \
                 then everything else grouped by when to do it. Be concise and practical.",
                crate::claude::DEFAULT_MODEL,
                now()
            ],
        )?;
    }
    Ok(())
}

// ---------- Todos ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub priority: i64,
    pub due_at: Option<String>,
    pub done: bool,
    pub created_at: String,
    pub done_at: Option<String>,
    pub project_id: Option<i64>,
    pub email_id: Option<i64>,
}

#[derive(Deserialize, Debug)]
pub struct NewTodo {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub due_at: Option<String>,
    #[serde(default)]
    pub project_id: Option<i64>,
}

fn default_priority() -> i64 {
    2
}

fn todo_from_row(r: &Row) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        priority: r.get(3)?,
        due_at: r.get(4)?,
        done: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
        done_at: r.get(7)?,
        project_id: r.get(8)?,
        email_id: r.get(9)?,
    })
}

const TODO_COLS: &str = "id, title, notes, priority, due_at, done, created_at, done_at, project_id, email_id";

pub fn list_todos(conn: &Connection) -> rusqlite::Result<Vec<Todo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TODO_COLS} FROM todos ORDER BY done ASC, priority ASC, due_at IS NULL, due_at ASC, id DESC"
    ))?;
    let rows = stmt.query_map([], todo_from_row)?;
    rows.collect()
}

pub fn add_todo(conn: &Connection, t: NewTodo) -> rusqlite::Result<Todo> {
    conn.execute(
        "INSERT INTO todos (title, notes, priority, due_at, created_at, project_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![t.title.trim(), t.notes, t.priority.clamp(1, 3), t.due_at, now(), t.project_id],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row(&format!("SELECT {TODO_COLS} FROM todos WHERE id = ?1"), [id], todo_from_row)
}

pub fn set_todo_done(conn: &Connection, id: i64, done: bool) -> rusqlite::Result<()> {
    let done_at = if done { Some(now()) } else { None };
    conn.execute(
        "UPDATE todos SET done = ?1, done_at = ?2 WHERE id = ?3",
        params![done as i64, done_at, id],
    )?;
    Ok(())
}

pub fn set_todo_priority(conn: &Connection, id: i64, priority: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE todos SET priority = ?1 WHERE id = ?2", params![priority.clamp(1, 3), id])?;
    Ok(())
}

pub fn delete_todo(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM todos WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- Reminders ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub remind_at: String,
    pub repeat: String,
    pub fired: bool,
    pub created_at: String,
    pub project_id: Option<i64>,
    pub email_id: Option<i64>,
}

#[derive(Deserialize, Debug)]
pub struct NewReminder {
    pub title: String,
    pub remind_at: String,
    #[serde(default = "default_repeat")]
    pub repeat: String,
    #[serde(default)]
    pub project_id: Option<i64>,
}

fn default_repeat() -> String {
    "none".into()
}

fn reminder_from_row(r: &Row) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: r.get(0)?,
        title: r.get(1)?,
        remind_at: r.get(2)?,
        repeat: r.get(3)?,
        fired: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
        project_id: r.get(6)?,
        email_id: r.get(7)?,
    })
}

const REMINDER_COLS: &str = "id, title, remind_at, repeat, fired, created_at, project_id, email_id";

pub fn list_reminders(conn: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REMINDER_COLS} FROM reminders ORDER BY fired ASC, remind_at ASC"
    ))?;
    let rows = stmt.query_map([], reminder_from_row)?;
    rows.collect()
}

pub fn add_reminder(conn: &Connection, r: NewReminder) -> Result<Reminder, String> {
    let at = chrono::DateTime::parse_from_rfc3339(&r.remind_at)
        .map_err(|e| format!("Invalid reminder time: {e}"))?
        .with_timezone(&chrono::Utc);
    let repeat = match r.repeat.as_str() {
        "none" | "daily" | "weekly" => r.repeat,
        other => return Err(format!("Unknown repeat value: {other}")),
    };
    conn.execute(
        "INSERT INTO reminders (title, remind_at, repeat, created_at, project_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![r.title.trim(), at.to_rfc3339(), repeat, now(), r.project_id],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        &format!("SELECT {REMINDER_COLS} FROM reminders WHERE id = ?1"),
        [id],
        reminder_from_row,
    )
    .map_err(|e| e.to_string())
}

pub fn delete_reminder(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM reminders WHERE id = ?1", [id])?;
    Ok(())
}

pub fn snooze_reminder(conn: &Connection, id: i64, minutes: i64) -> rusqlite::Result<()> {
    let at = chrono::Utc::now() + chrono::Duration::minutes(minutes);
    conn.execute(
        "UPDATE reminders SET remind_at = ?1, fired = 0 WHERE id = ?2",
        params![at.to_rfc3339(), id],
    )?;
    Ok(())
}

/// Returns reminders that are due and not yet fired, and advances or marks them.
pub fn take_due_reminders(conn: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    let now = chrono::Utc::now();
    let mut stmt = conn.prepare(&format!(
        "SELECT {REMINDER_COLS} FROM reminders WHERE fired = 0 AND remind_at <= ?1"
    ))?;
    let due: Vec<Reminder> = stmt
        .query_map([now.to_rfc3339()], reminder_from_row)?
        .collect::<rusqlite::Result<_>>()?;

    for r in &due {
        let step = match r.repeat.as_str() {
            "daily" => Some(chrono::Duration::days(1)),
            "weekly" => Some(chrono::Duration::weeks(1)),
            _ => None,
        };
        match step {
            Some(step) => {
                // Move to the next occurrence in the future, skipping any missed ones.
                let mut next = chrono::DateTime::parse_from_rfc3339(&r.remind_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or(now);
                while next <= now {
                    next += step;
                }
                conn.execute(
                    "UPDATE reminders SET remind_at = ?1 WHERE id = ?2",
                    params![next.to_rfc3339(), r.id],
                )?;
            }
            None => {
                conn.execute("UPDATE reminders SET fired = 1 WHERE id = ?1", [r.id])?;
            }
        }
    }
    Ok(due)
}

// ---------- Projects ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub color: String,
    pub archived: bool,
    pub created_at: String,
}

#[derive(Deserialize, Debug)]
pub struct ProjectInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub archived: bool,
}

fn default_color() -> String {
    "#4c8dff".into()
}

fn project_from_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        color: r.get(3)?,
        archived: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
    })
}

const PROJECT_COLS: &str = "id, name, description, color, archived, created_at";

pub fn list_projects(conn: &Connection) -> rusqlite::Result<Vec<Project>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJECT_COLS} FROM projects ORDER BY archived ASC, name COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([], project_from_row)?;
    rows.collect()
}

pub fn save_project(conn: &Connection, p: ProjectInput) -> rusqlite::Result<Project> {
    let id = match p.id {
        Some(id) => {
            conn.execute(
                "UPDATE projects SET name = ?1, description = ?2, color = ?3, archived = ?4 WHERE id = ?5",
                params![p.name.trim(), p.description, p.color, p.archived as i64, id],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO projects (name, description, color, archived, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.name.trim(), p.description, p.color, p.archived as i64, now()],
            )?;
            conn.last_insert_rowid()
        }
    };
    conn.query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"), [id], project_from_row)
}

/// Deletes a project. Its tasks and reminders are kept, unassigned.
pub fn delete_project(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- Settings ----------

pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------- Mail ----------

#[derive(Serialize, Clone, Debug)]
pub struct MailAccount {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    pub client_id: String,
    pub tenant_id: String,
    pub summary: String,
    pub summary_at: Option<String>,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub unread: i64,
    pub pending: i64,
}

const ACCOUNT_SELECT: &str = "SELECT a.id, a.email, a.display_name, a.client_id, a.tenant_id, a.summary, a.summary_at,
        a.last_sync_at, a.last_error,
        (SELECT COUNT(*) FROM emails e WHERE e.account_id = a.id AND e.is_read = 0),
        (SELECT COUNT(*) FROM suggestions s WHERE s.account_id = a.id AND s.status = 'pending')
     FROM mail_accounts a";

fn account_from_row(r: &Row) -> rusqlite::Result<MailAccount> {
    Ok(MailAccount {
        id: r.get(0)?,
        email: r.get(1)?,
        display_name: r.get(2)?,
        client_id: r.get(3)?,
        tenant_id: r.get(4)?,
        summary: r.get(5)?,
        summary_at: r.get(6)?,
        last_sync_at: r.get(7)?,
        last_error: r.get(8)?,
        unread: r.get(9)?,
        pending: r.get(10)?,
    })
}

pub fn list_accounts(conn: &Connection) -> rusqlite::Result<Vec<MailAccount>> {
    let mut stmt = conn.prepare(&format!("{ACCOUNT_SELECT} ORDER BY a.email"))?;
    let rows = stmt.query_map([], account_from_row)?;
    rows.collect()
}

pub fn get_account(conn: &Connection, id: i64) -> rusqlite::Result<Option<MailAccount>> {
    conn.query_row(&format!("{ACCOUNT_SELECT} WHERE a.id = ?1"), [id], account_from_row).optional()
}

/// Adds an account, or updates its app registration if it is signed in again.
pub fn upsert_account(conn: &Connection, email: &str, display_name: &str, client_id: &str, tenant_id: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO mail_accounts (email, display_name, client_id, tenant_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(email) DO UPDATE SET display_name = excluded.display_name, client_id = excluded.client_id,
            tenant_id = excluded.tenant_id, last_error = NULL",
        params![email, display_name, client_id, tenant_id, now()],
    )?;
    conn.query_row("SELECT id FROM mail_accounts WHERE email = ?1", [email], |r| r.get(0))
}

pub fn delete_account(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM mail_accounts WHERE id = ?1", [id])?;
    Ok(())
}

pub fn set_sync_result(conn: &Connection, id: i64, error: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE mail_accounts SET last_sync_at = ?1, last_error = ?2 WHERE id = ?3",
        params![now(), error, id],
    )?;
    Ok(())
}

pub fn set_summary(conn: &Connection, id: i64, summary: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE mail_accounts SET summary = ?1, summary_at = ?2 WHERE id = ?3",
        params![summary, now(), id],
    )?;
    Ok(())
}

#[derive(Serialize, Clone, Debug)]
pub struct Email {
    pub id: i64,
    pub account_id: i64,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub received_at: String,
    pub preview: String,
    pub is_read: bool,
    pub web_link: String,
    pub analyzed: bool,
}

#[derive(Debug, Clone)]
pub struct NewEmail {
    pub remote_id: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub received_at: String,
    pub preview: String,
    pub body: String,
    pub is_read: bool,
    pub web_link: String,
}

const EMAIL_COLS: &str = "id, account_id, subject, from_name, from_addr, received_at, preview, is_read, web_link, analyzed";

fn email_from_row(r: &Row) -> rusqlite::Result<Email> {
    Ok(Email {
        id: r.get(0)?,
        account_id: r.get(1)?,
        subject: r.get(2)?,
        from_name: r.get(3)?,
        from_addr: r.get(4)?,
        received_at: r.get(5)?,
        preview: r.get(6)?,
        is_read: r.get::<_, i64>(7)? != 0,
        web_link: r.get(8)?,
        analyzed: r.get::<_, i64>(9)? != 0,
    })
}

/// Stores fetched messages, refreshing read state on ones already stored.
/// Returns how many were new. Keeps only the newest `keep` per account.
pub fn store_emails(conn: &Connection, account_id: i64, emails: &[NewEmail], keep: i64) -> rusqlite::Result<usize> {
    let mut added = 0;
    for e in emails {
        added += conn.execute(
            "INSERT OR IGNORE INTO emails (account_id, remote_id, subject, from_name, from_addr, received_at, preview, body, is_read, web_link)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![account_id, e.remote_id, e.subject, e.from_name, e.from_addr, e.received_at, e.preview, e.body, e.is_read as i64, e.web_link],
        )?;
        conn.execute(
            "UPDATE emails SET is_read = ?1 WHERE account_id = ?2 AND remote_id = ?3",
            params![e.is_read as i64, account_id, e.remote_id],
        )?;
    }
    conn.execute(
        "DELETE FROM emails WHERE account_id = ?1 AND id NOT IN
            (SELECT id FROM emails WHERE account_id = ?1 ORDER BY received_at DESC LIMIT ?2)",
        params![account_id, keep],
    )?;
    Ok(added)
}

pub fn list_emails(conn: &Connection, account_id: i64, limit: i64) -> rusqlite::Result<Vec<Email>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EMAIL_COLS} FROM emails WHERE account_id = ?1 ORDER BY received_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![account_id, limit], email_from_row)?;
    rows.collect()
}

/// The newest emails with their bodies, for Claude to read.
pub fn emails_for_analysis(conn: &Connection, account_id: i64, limit: i64) -> rusqlite::Result<Vec<(Email, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EMAIL_COLS}, body FROM emails WHERE account_id = ?1 ORDER BY received_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![account_id, limit], |r| Ok((email_from_row(r)?, r.get(10)?)))?;
    rows.collect()
}

pub fn mark_analyzed(conn: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    for id in ids {
        conn.execute("UPDATE emails SET analyzed = 1 WHERE id = ?1", [id])?;
    }
    Ok(())
}

#[derive(Serialize, Clone, Debug)]
pub struct Suggestion {
    pub id: i64,
    pub account_id: i64,
    pub email_id: Option<i64>,
    pub kind: String,
    pub title: String,
    pub notes: String,
    pub due_at: Option<String>,
    pub priority: i64,
    pub project_id: Option<i64>,
    pub status: String,
    pub created_at: String,
    pub email_subject: Option<String>,
    pub email_from: Option<String>,
    pub email_link: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewSuggestion {
    pub account_id: i64,
    pub email_id: i64,
    pub kind: String,
    pub title: String,
    pub notes: String,
    pub due_at: Option<String>,
    pub priority: i64,
    pub project_id: Option<i64>,
}

pub fn add_suggestion(conn: &Connection, s: &NewSuggestion) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO suggestions (account_id, email_id, kind, title, notes, due_at, priority, project_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![s.account_id, s.email_id, s.kind, s.title.trim(), s.notes, s.due_at, s.priority.clamp(1, 3), s.project_id, now()],
    )?;
    Ok(())
}

pub fn list_pending_suggestions(conn: &Connection) -> rusqlite::Result<Vec<Suggestion>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.account_id, s.email_id, s.kind, s.title, s.notes, s.due_at, s.priority, s.project_id, s.status, s.created_at,
                e.subject, COALESCE(NULLIF(e.from_name, ''), e.from_addr), e.web_link
         FROM suggestions s LEFT JOIN emails e ON e.id = s.email_id
         WHERE s.status = 'pending' ORDER BY s.id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Suggestion {
            id: r.get(0)?,
            account_id: r.get(1)?,
            email_id: r.get(2)?,
            kind: r.get(3)?,
            title: r.get(4)?,
            notes: r.get(5)?,
            due_at: r.get(6)?,
            priority: r.get(7)?,
            project_id: r.get(8)?,
            status: r.get(9)?,
            created_at: r.get(10)?,
            email_subject: r.get(11)?,
            email_from: r.get(12)?,
            email_link: r.get(13)?,
        })
    })?;
    rows.collect()
}

pub fn dismiss_suggestion(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE suggestions SET status = 'dismissed' WHERE id = ?1", [id])?;
    Ok(())
}

/// What the user may change on a suggestion before accepting it.
#[derive(Deserialize, Debug)]
pub struct AcceptSuggestion {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub due_at: Option<String>,
    pub project_id: Option<i64>,
}

/// Turns a suggestion into a real todo or reminder, linked to its email.
pub fn accept_suggestion(conn: &Connection, a: AcceptSuggestion) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    let (email_id, notes, priority): (Option<i64>, String, i64) = conn
        .query_row("SELECT email_id, notes, priority FROM suggestions WHERE id = ?1 AND status = 'pending'", [a.id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .map_err(e)?
        .ok_or("That suggestion was already handled.")?;
    if a.title.trim().is_empty() {
        return Err("A suggestion needs a title before it can be added.".into());
    }
    match a.kind.as_str() {
        "todo" => {
            let t = add_todo(conn, NewTodo { title: a.title, notes, priority, due_at: a.due_at, project_id: a.project_id }).map_err(e)?;
            conn.execute("UPDATE todos SET email_id = ?1 WHERE id = ?2", params![email_id, t.id]).map_err(e)?;
        }
        "reminder" => {
            let at = a.due_at.ok_or("Pick a time for the reminder first.")?;
            let r = add_reminder(conn, NewReminder { title: a.title, remind_at: at, repeat: "none".into(), project_id: a.project_id })?;
            conn.execute("UPDATE reminders SET email_id = ?1 WHERE id = ?2", params![email_id, r.id]).map_err(e)?;
        }
        other => return Err(format!("Unknown suggestion type: {other}")),
    }
    conn.execute("UPDATE suggestions SET status = 'accepted' WHERE id = ?1", [a.id]).map_err(e)?;
    Ok(())
}

// ---------- Agents ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Agent {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub model: String,
    pub created_at: String,
}

#[derive(Deserialize, Debug)]
pub struct AgentInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    pub model: String,
}

fn agent_from_row(r: &Row) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        system_prompt: r.get(3)?,
        model: r.get(4)?,
        created_at: r.get(5)?,
    })
}

const AGENT_COLS: &str = "id, name, description, system_prompt, model, created_at";

pub fn list_agents(conn: &Connection) -> rusqlite::Result<Vec<Agent>> {
    let mut stmt = conn.prepare(&format!("SELECT {AGENT_COLS} FROM agents ORDER BY name"))?;
    let rows = stmt.query_map([], agent_from_row)?;
    rows.collect()
}

pub fn get_agent(conn: &Connection, id: i64) -> rusqlite::Result<Option<Agent>> {
    conn.query_row(
        &format!("SELECT {AGENT_COLS} FROM agents WHERE id = ?1"),
        [id],
        agent_from_row,
    )
    .optional()
}

pub fn save_agent(conn: &Connection, a: AgentInput) -> rusqlite::Result<Agent> {
    let id = match a.id {
        Some(id) => {
            conn.execute(
                "UPDATE agents SET name = ?1, description = ?2, system_prompt = ?3, model = ?4 WHERE id = ?5",
                params![a.name.trim(), a.description, a.system_prompt, a.model, id],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO agents (name, description, system_prompt, model, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![a.name.trim(), a.description, a.system_prompt, a.model, now()],
            )?;
            conn.last_insert_rowid()
        }
    };
    conn.query_row(&format!("SELECT {AGENT_COLS} FROM agents WHERE id = ?1"), [id], agent_from_row)
}

pub fn delete_agent(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM agents WHERE id = ?1", [id])?;
    Ok(())
}

#[derive(Serialize, Clone, Debug)]
pub struct AgentRun {
    pub id: i64,
    pub agent_id: i64,
    pub agent_name: String,
    pub input: String,
    pub output: String,
    pub status: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub fn start_run(conn: &Connection, agent_id: i64, input: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO agent_runs (agent_id, input, status, started_at) VALUES (?1, ?2, 'running', ?3)",
        params![agent_id, input, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_run(
    conn: &Connection,
    run_id: i64,
    status: &str,
    output: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_runs SET status = ?1, output = ?2, model = ?3, input_tokens = ?4, output_tokens = ?5, finished_at = ?6 WHERE id = ?7",
        params![status, output, model, input_tokens, output_tokens, now(), run_id],
    )?;
    Ok(())
}

pub fn list_runs(conn: &Connection, agent_id: Option<i64>, limit: i64) -> rusqlite::Result<Vec<AgentRun>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.agent_id, a.name, r.input, r.output, r.status, r.model, r.input_tokens, r.output_tokens, r.started_at, r.finished_at
         FROM agent_runs r JOIN agents a ON a.id = r.agent_id
         WHERE (?1 IS NULL OR r.agent_id = ?1)
         ORDER BY r.id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![agent_id, limit], |r| {
        Ok(AgentRun {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            agent_name: r.get(2)?,
            input: r.get(3)?,
            output: r.get(4)?,
            status: r.get(5)?,
            model: r.get(6)?,
            input_tokens: r.get(7)?,
            output_tokens: r.get(8)?,
            started_at: r.get(9)?,
            finished_at: r.get(10)?,
        })
    })?;
    rows.collect()
}

/// Runs left as "running" when the app last closed never finished.
pub fn mark_orphaned_runs(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_runs SET status = 'error', output = 'Interrupted: the app closed before this run finished.', finished_at = ?1 WHERE status = 'running'",
        [now()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        migrate(&conn).unwrap();
        seed_default_agent(&conn).unwrap();
        conn
    }

    #[test]
    fn project_tasks_survive_project_delete() {
        let c = mem();
        let p = save_project(&c, ProjectInput { id: None, name: " Website ".into(), description: "".into(), color: default_color(), archived: false }).unwrap();
        assert_eq!(p.name, "Website");
        let t = add_todo(&c, NewTodo { title: "Draft copy".into(), notes: "".into(), priority: 2, due_at: None, project_id: Some(p.id) }).unwrap();
        assert_eq!(t.project_id, Some(p.id));
        delete_project(&c, p.id).unwrap();
        assert!(list_projects(&c).unwrap().is_empty());
        assert_eq!(list_todos(&c).unwrap()[0].project_id, None);
    }

    fn sample_email(remote_id: &str, received_at: &str) -> NewEmail {
        NewEmail {
            remote_id: remote_id.into(),
            subject: format!("Subject {remote_id}"),
            from_name: "Pat".into(),
            from_addr: "pat@example.com".into(),
            received_at: received_at.into(),
            preview: "Can you send the quote by Friday?".into(),
            body: "Can you send the quote by Friday?".into(),
            is_read: false,
            web_link: "https://outlook.office.com/x".into(),
        }
    }

    #[test]
    fn mail_sync_dedupes_and_prunes() {
        let c = mem();
        let a = upsert_account(&c, "me@example.com", "Me", "client", "tenant").unwrap();
        assert_eq!(upsert_account(&c, "me@example.com", "Me", "client2", "tenant").unwrap(), a);
        let batch = vec![sample_email("1", "2026-09-01T10:00:00Z"), sample_email("2", "2026-09-02T10:00:00Z")];
        assert_eq!(store_emails(&c, a, &batch, 100).unwrap(), 2);
        let mut again = batch.clone();
        again[0].is_read = true;
        assert_eq!(store_emails(&c, a, &again, 100).unwrap(), 0);
        let acct = get_account(&c, a).unwrap().unwrap();
        assert_eq!((acct.unread, acct.client_id.as_str()), (1, "client2"));
        store_emails(&c, a, &[sample_email("3", "2026-09-03T10:00:00Z")], 2).unwrap();
        let kept: Vec<String> = list_emails(&c, a, 10).unwrap().into_iter().map(|e| e.subject).collect();
        assert_eq!(kept, vec!["Subject 3", "Subject 2"]);
    }

    #[test]
    fn accepting_a_suggestion_creates_a_linked_todo() {
        let c = mem();
        let a = upsert_account(&c, "me@example.com", "Me", "client", "tenant").unwrap();
        store_emails(&c, a, &[sample_email("1", "2026-09-01T10:00:00Z")], 100).unwrap();
        let email_id = list_emails(&c, a, 1).unwrap()[0].id;
        let s = NewSuggestion { account_id: a, email_id, kind: "todo".into(), title: "Send quote".into(), notes: "".into(), due_at: None, priority: 1, project_id: None };
        add_suggestion(&c, &s).unwrap();
        add_suggestion(&c, &NewSuggestion { kind: "reminder".into(), ..s }).unwrap();
        let pending = list_pending_suggestions(&c).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].email_from.as_deref(), Some("Pat"));

        let todo_id = pending.iter().find(|p| p.kind == "todo").unwrap().id;
        accept_suggestion(&c, AcceptSuggestion { id: todo_id, kind: "todo".into(), title: "Send quote".into(), due_at: None, project_id: None }).unwrap();
        let todos = list_todos(&c).unwrap();
        assert_eq!((todos[0].email_id, todos[0].priority), (Some(email_id), 1));
        assert!(accept_suggestion(&c, AcceptSuggestion { id: todo_id, kind: "todo".into(), title: "x".into(), due_at: None, project_id: None }).is_err());

        let rem_id = pending.iter().find(|p| p.kind == "reminder").unwrap().id;
        assert!(accept_suggestion(&c, AcceptSuggestion { id: rem_id, kind: "reminder".into(), title: "x".into(), due_at: None, project_id: None }).is_err());
        dismiss_suggestion(&c, rem_id).unwrap();
        assert!(list_pending_suggestions(&c).unwrap().is_empty());
    }

    #[test]
    fn migrates_existing_database() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT); CREATE TABLE reminders (id INTEGER PRIMARY KEY, title TEXT);").unwrap();
        c.execute_batch(SCHEMA).unwrap();
        migrate(&c).unwrap();
        assert!(has_column(&c, "todos", "project_id").unwrap());
        assert!(has_column(&c, "reminders", "project_id").unwrap());
        migrate(&c).unwrap();
    }

    #[test]
    fn todos_roundtrip() {
        let c = mem();
        let t = add_todo(&c, NewTodo { title: " Call lab ".into(), notes: "".into(), priority: 9, due_at: None, project_id: None }).unwrap();
        assert_eq!(t.title, "Call lab");
        assert_eq!(t.priority, 3);
        set_todo_done(&c, t.id, true).unwrap();
        assert!(list_todos(&c).unwrap()[0].done);
        delete_todo(&c, t.id).unwrap();
        assert!(list_todos(&c).unwrap().is_empty());
    }

    #[test]
    fn one_off_reminder_fires_once() {
        let c = mem();
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        add_reminder(&c, NewReminder { title: "x".into(), remind_at: past, repeat: "none".into(), project_id: None }).unwrap();
        assert_eq!(take_due_reminders(&c).unwrap().len(), 1);
        assert_eq!(take_due_reminders(&c).unwrap().len(), 0);
        assert!(list_reminders(&c).unwrap()[0].fired);
    }

    #[test]
    fn daily_reminder_rolls_forward() {
        let c = mem();
        let past = (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        add_reminder(&c, NewReminder { title: "x".into(), remind_at: past, repeat: "daily".into(), project_id: None }).unwrap();
        assert_eq!(take_due_reminders(&c).unwrap().len(), 1);
        let r = &list_reminders(&c).unwrap()[0];
        assert!(!r.fired);
        let next = chrono::DateTime::parse_from_rfc3339(&r.remind_at).unwrap();
        assert!(next > chrono::Utc::now());
        assert_eq!(take_due_reminders(&c).unwrap().len(), 0);
    }

    #[test]
    fn rejects_bad_reminder_input() {
        let c = mem();
        assert!(add_reminder(&c, NewReminder { title: "x".into(), remind_at: "tomorrow".into(), repeat: "none".into(), project_id: None }).is_err());
        let t = chrono::Utc::now().to_rfc3339();
        assert!(add_reminder(&c, NewReminder { title: "x".into(), remind_at: t, repeat: "hourly".into(), project_id: None }).is_err());
    }

    #[test]
    fn default_agent_and_runs() {
        let c = mem();
        let agents = list_agents(&c).unwrap();
        assert_eq!(agents.len(), 1);
        let run = start_run(&c, agents[0].id, "hi").unwrap();
        mark_orphaned_runs(&c).unwrap();
        let runs = list_runs(&c, None, 10).unwrap();
        assert_eq!(runs[0].id, run);
        assert_eq!(runs[0].status, "error");
    }
}
