//! Local SQLite storage for todos, reminders, agents and agent runs.

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
        seed_default_agent(&conn)?;
        Ok(Db(Mutex::new(conn)))
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
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
}

#[derive(Deserialize, Debug)]
pub struct NewTodo {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_priority")]
    pub priority: i64,
    pub due_at: Option<String>,
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
    })
}

const TODO_COLS: &str = "id, title, notes, priority, due_at, done, created_at, done_at";

pub fn list_todos(conn: &Connection) -> rusqlite::Result<Vec<Todo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TODO_COLS} FROM todos ORDER BY done ASC, priority ASC, due_at IS NULL, due_at ASC, id DESC"
    ))?;
    let rows = stmt.query_map([], todo_from_row)?;
    rows.collect()
}

pub fn add_todo(conn: &Connection, t: NewTodo) -> rusqlite::Result<Todo> {
    conn.execute(
        "INSERT INTO todos (title, notes, priority, due_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![t.title.trim(), t.notes, t.priority.clamp(1, 3), t.due_at, now()],
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
}

#[derive(Deserialize, Debug)]
pub struct NewReminder {
    pub title: String,
    pub remind_at: String,
    #[serde(default = "default_repeat")]
    pub repeat: String,
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
    })
}

const REMINDER_COLS: &str = "id, title, remind_at, repeat, fired, created_at";

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
        "INSERT INTO reminders (title, remind_at, repeat, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![r.title.trim(), at.to_rfc3339(), repeat, now()],
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
        seed_default_agent(&conn).unwrap();
        conn
    }

    #[test]
    fn todos_roundtrip() {
        let c = mem();
        let t = add_todo(&c, NewTodo { title: " Call lab ".into(), notes: "".into(), priority: 9, due_at: None }).unwrap();
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
        add_reminder(&c, NewReminder { title: "x".into(), remind_at: past, repeat: "none".into() }).unwrap();
        assert_eq!(take_due_reminders(&c).unwrap().len(), 1);
        assert_eq!(take_due_reminders(&c).unwrap().len(), 0);
        assert!(list_reminders(&c).unwrap()[0].fired);
    }

    #[test]
    fn daily_reminder_rolls_forward() {
        let c = mem();
        let past = (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        add_reminder(&c, NewReminder { title: "x".into(), remind_at: past, repeat: "daily".into() }).unwrap();
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
        assert!(add_reminder(&c, NewReminder { title: "x".into(), remind_at: "tomorrow".into(), repeat: "none".into() }).is_err());
        let t = chrono::Utc::now().to_rfc3339();
        assert!(add_reminder(&c, NewReminder { title: "x".into(), remind_at: t, repeat: "hourly".into() }).is_err());
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
