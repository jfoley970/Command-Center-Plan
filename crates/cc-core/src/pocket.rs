//! Voice commands from a Pocket recorder (heypocket.com).
//!
//! Press record, say "Command Center, remind me to call Dana at 3 PM", stop.
//! Once Pocket has transcribed the recording, this module finds every command
//! that follows the wake phrase and turns it into a todo, a reminder or an
//! agent request. Recordings arrive two ways, and both may be on at once:
//!
//! - Polling: the backend lists new recordings every two minutes with the
//!   Pocket API key. Works anywhere, including the desktop app.
//! - Webhook: on the server, Pocket can POST to `/hooks/pocket` the moment a
//!   transcript is ready. Requests are verified with Pocket's HMAC signature.
//!
//! Simple commands are read with fixed rules. When the rules can't tell what a
//! command means (or when a reminder has no time they recognise), the local
//! model in Ollama reads it instead, so the words never leave the server.
//! Without Ollama the rules' best guess stands.
//!
//! Only recordings that start with the wake phrase are acted on, and only the
//! command words are stored. Transcripts of every other recording are read,
//! found to hold no command, and dropped; their ids are kept for 30 days so the
//! same recording isn't read twice.

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, LazyLock, Mutex};

use crate::secrets::{POCKET_KEY, POCKET_WEBHOOK_SECRET};
use crate::{commands, db, ollama, providers, Core};

const API_URL: &str = "https://public.heypocketai.com/api/v1/public/recordings";
const POLL_SECS: u64 = 120;
const PAGE_SIZE: u32 = 50;
const MAX_PAGES: u32 = 5;
/// A recording without a transcript yet is retried for this long.
const TRANSCRIPT_WAIT_HOURS: i64 = 6;
/// Pocket's signature timestamp must be this fresh, so a captured request can't be replayed later.
const WEBHOOK_MAX_AGE_MS: i64 = 5 * 60 * 1000;
/// Longest command kept; anything past this was probably not meant as one.
const MAX_COMMAND_CHARS: usize = 500;

pub const DEFAULT_WAKE: &str = "command center";

const SET_WAKE: &str = "pocket_wake";
const SET_AUTO: &str = "pocket_auto";
const SET_SINCE: &str = "pocket_since";
const SET_AI: &str = "pocket_ai";
const SET_AI_MODEL: &str = "pocket_ai_model";
/// How long a local model may take over one command (it may have to load first).
const AI_TIMEOUT_SECS: u64 = 120;

// ---------- Storage ----------

#[derive(Serialize, Clone, Debug)]
pub struct Command {
    pub id: i64,
    pub recording_id: String,
    pub recorded_at: Option<String>,
    /// The words after the wake phrase, as transcribed.
    pub heard: String,
    /// "todo", "reminder" or "agent".
    pub kind: String,
    pub title: String,
    pub due_at: Option<String>,
    /// For agent requests: "agent:<id>" or "provider:<id>".
    pub target: String,
    /// Who handles an agent request, for display.
    pub target_name: String,
    /// "pending", "done", "dismissed" or "error".
    pub status: String,
    pub result: String,
    pub created_at: String,
    /// "rules" or "local" (the local model read it).
    pub via: String,
}

const COLS: &str = "id, recording_id, recorded_at, heard, kind, title, due_at, target, target_name, status, result, created_at, via";

fn from_row(r: &Row) -> rusqlite::Result<Command> {
    Ok(Command {
        id: r.get(0)?,
        recording_id: r.get(1)?,
        recorded_at: r.get(2)?,
        heard: r.get(3)?,
        kind: r.get(4)?,
        title: r.get(5)?,
        due_at: r.get(6)?,
        target: r.get(7)?,
        target_name: r.get(8)?,
        status: r.get(9)?,
        result: r.get(10)?,
        created_at: r.get(11)?,
        via: r.get(12)?,
    })
}

pub fn list(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<Command>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM pocket_commands ORDER BY status != 'pending', id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], from_row)?;
    rows.collect()
}

fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Command>> {
    conn.query_row(&format!("SELECT {COLS} FROM pocket_commands WHERE id = ?1"), [id], from_row).optional()
}

fn finish(conn: &Connection, id: i64, status: &str, result: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE pocket_commands SET status = ?2, result = ?3 WHERE id = ?1", params![id, status, result])?;
    Ok(())
}

/// Marks a recording as read. Returns false if it already was, so a recording
/// that arrives by webhook and by polling at the same time runs only once.
fn claim(conn: &Connection, recording_id: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO pocket_seen (recording_id, seen_at) VALUES (?1, ?2)",
        params![recording_id, db::now()],
    )?;
    Ok(n == 1)
}

fn is_seen(conn: &Connection, recording_id: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM pocket_seen WHERE recording_id = ?1", [recording_id], |_| Ok(()))
        .optional()
        .map(|r| r.is_some())
}

fn prune_seen(conn: &Connection) -> rusqlite::Result<()> {
    let cutoff = (Utc::now() - Duration::days(30)).to_rfc3339();
    conn.execute("DELETE FROM pocket_seen WHERE seen_at < ?1", [cutoff])?;
    Ok(())
}

// ---------- Options ----------

/// Which commands run without a tap.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Auto {
    /// Todos and timed reminders run at once; agent requests wait for approval.
    Tasks,
    /// Agent requests run too.
    All,
    /// Everything waits for approval.
    None,
}

impl Auto {
    fn parse(s: Option<String>) -> Self {
        match s.as_deref() {
            Some("all") => Auto::All,
            Some("none") => Auto::None,
            _ => Auto::Tasks,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Auto::Tasks => "tasks",
            Auto::All => "all",
            Auto::None => "none",
        }
    }
    fn runs(self, p: &Parsed) -> bool {
        match p.kind.as_str() {
            "reminder" if p.due_at.is_none() => false,
            "agent" => self == Auto::All,
            _ => self != Auto::None,
        }
    }
}

fn setting(conn: &Connection, key: &str) -> Option<String> {
    db::get_setting(conn, key).ok().flatten().filter(|v| !v.trim().is_empty())
}

fn ai_on(conn: &Connection) -> bool {
    setting(conn, SET_AI).as_deref() != Some("off")
}

pub fn set_options(core: &Core, wake: &str, auto: Auto, ai: bool, ai_model: &str) -> Result<(), String> {
    let wake = wake.trim();
    if words(wake).is_empty() {
        return Err("The wake phrase needs at least one word.".into());
    }
    if wake.len() > 40 {
        return Err("Keep the wake phrase short, a word or two.".into());
    }
    let conn = core.db.0.lock().unwrap();
    db::set_setting(&conn, SET_WAKE, wake).map_err(|e| e.to_string())?;
    db::set_setting(&conn, SET_AUTO, auto.as_str()).map_err(|e| e.to_string())?;
    db::set_setting(&conn, SET_AI, if ai { "local" } else { "off" }).map_err(|e| e.to_string())?;
    db::set_setting(&conn, SET_AI_MODEL, ai_model.trim()).map_err(|e| e.to_string())?;
    drop(conn);
    core.changed("pocket-changed");
    Ok(())
}

/// Called when a key or webhook secret is first saved: only recordings made
/// from now on are read, so connecting doesn't replay old ones.
pub fn mark_connected(core: &Core) -> Result<(), String> {
    let conn = core.db.0.lock().unwrap();
    if setting(&conn, SET_SINCE).is_none() {
        db::set_setting(&conn, SET_SINCE, &Utc::now().to_rfc3339()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- Finding commands in a transcript ----------

/// Lowercase words without punctuation, keeping where each started in the text.
fn words(text: &str) -> Vec<(usize, String)> {
    let mut out = vec![];
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        let part_of_word = c.is_alphanumeric() || c == '\'';
        match (part_of_word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let w: String = text[s..i].chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
                // Pocket may spell it either way.
                let w = if w == "centre" { "center".to_string() } else { w };
                if !w.is_empty() {
                    out.push((s, w));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Every command in a transcript. The recording must open with the wake phrase
/// (a word or two such as "OK" or "hey" before it is fine); each later "Command
/// Center" starts another command.
pub fn extract(transcript: &str, wake: &str) -> Vec<String> {
    let wake: Vec<String> = words(wake).into_iter().map(|(_, w)| w).collect();
    let ws = words(transcript);
    if wake.is_empty() {
        return vec![];
    }
    // Also accept the phrase run together, such as "commandcenter".
    let joined = wake.concat();
    let match_at = |i: usize| -> Option<usize> {
        if ws.len() >= i + wake.len() && ws[i..i + wake.len()].iter().map(|(_, w)| w).eq(wake.iter()) {
            Some(wake.len())
        } else if wake.len() > 1 && ws.get(i).is_some_and(|(_, w)| *w == joined) {
            Some(1)
        } else {
            None
        }
    };
    let Some((first, len)) = (0..ws.len().min(3)).find_map(|i| match_at(i).map(|n| (i, n))) else {
        return vec![];
    };

    // Byte ranges of each command: from after one wake phrase to the next.
    let mut bounds = vec![];
    let mut cmd_start = first + len;
    let mut i = cmd_start;
    while i < ws.len() {
        if let Some(n) = match_at(i) {
            bounds.push((cmd_start, i));
            cmd_start = i + n;
            i = cmd_start;
        } else {
            i += 1;
        }
    }
    bounds.push((cmd_start, ws.len()));

    bounds
        .into_iter()
        .filter(|(a, b)| a < b)
        .map(|(a, b)| {
            let from = ws[a].0;
            let to = ws.get(b).map_or(transcript.len(), |(s, _)| *s);
            tidy(&transcript[from..to]).chars().take(MAX_COMMAND_CHARS).collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn tidy(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.trim_matches(|c: char| !c.is_alphanumeric() && c != '?' && c != ')' && c != '"' && c != '\'').to_string()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

// ---------- Working out what a command means ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub kind: String,
    pub title: String,
    pub due_at: Option<String>,
    pub target: String,
    pub target_name: String,
    /// False when the rules only guessed (an unrecognised command became a
    /// todo, or a reminder has no time), so the local model should read it.
    pub sure: bool,
}

static REMIND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:please\s+)?(?:remind\s+me|(?:set|create|add)\s+(?:a\s+)?reminder|reminder)\b[\s,:.\-]*(?:(?:to|that|about)\s+)?").unwrap()
});
static TODO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:add|create|new|make)\s+(?:a\s+|an\s+|new\s+)?)?(?:to[\s\-]?do|task)(?:\s+item)?\b[\s,:.\-]*(?:(?:to|that)\s+)?").unwrap()
});
static ADD_TO_LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^add\s+(.+?)\s+to\s+(?:my\s+|the\s+)?(?:to[\s\-]?do\s+|task\s+)?list$").unwrap());
static ASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:ask|tell|have)\s+").unwrap());
static IN_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bin\s+(half\s+an\s+hour|an?|\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|fifteen|twenty|thirty|forty[\s\-]five|forty|fifty|ninety)\s*(minutes?|mins?|hours?|hrs?|days?)?\b").unwrap()
});
static AT_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b(today|tomorrow|tonight)\s+)?\bat\s+(noon|midnight|\d{1,2})(?::(\d{2}))?\s*(a\.?\s?m\.?|p\.?\s?m\.?)?(?:\s+(today|tomorrow|tonight))?").unwrap()
});
static DAY_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(tomorrow|tonight)(?:\s+(morning|afternoon|evening))?\b").unwrap());

fn number(word: &str) -> Option<i64> {
    let w = word.to_lowercase().replace('-', " ");
    let w = w.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(match w.as_str() {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "fifteen" => 15,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "forty five" => 45,
        "fifty" => 50,
        "ninety" => 90,
        _ => w.parse().ok()?,
    })
}

fn local_at(day: chrono::NaiveDate, h: u32, m: u32) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(h, m, 0)?)).earliest()
}

/// Finds a spoken time in a reminder and returns it with the text minus the time.
fn when(text: &str, now: DateTime<Local>) -> (Option<DateTime<Local>>, String) {
    let cut = |m: regex::Match| format!("{} {}", &text[..m.start()], &text[m.end()..]);

    if let Some(c) = IN_TIME.captures(text) {
        let amount = c.get(1).unwrap().as_str();
        let unit = c.get(2).map(|u| u.as_str().to_lowercase());
        let minutes = if amount.to_lowercase().starts_with("half") {
            Some(30)
        } else {
            number(amount).and_then(|n| match unit.as_deref() {
                Some(u) if u.starts_with("min") => Some(n),
                Some(u) if u.starts_with('h') => Some(n * 60),
                Some(u) if u.starts_with('d') => Some(n * 60 * 24),
                _ => None,
            })
        };
        if let Some(min) = minutes.filter(|m| (1..=60 * 24 * 60).contains(m)) {
            return (Some(now + Duration::minutes(min)), cut(c.get(0).unwrap()));
        }
    }

    if let Some(c) = AT_TIME.captures(text) {
        let day_word = c.get(1).or(c.get(5)).map(|d| d.as_str().to_lowercase());
        let hour_word = c.get(2).unwrap().as_str().to_lowercase();
        let minute: u32 = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let meridiem = c.get(4).map(|m| m.as_str().to_lowercase().starts_with('p'));
        let hour: Option<u32> = match hour_word.as_str() {
            "noon" => Some(12),
            "midnight" => Some(0),
            h => h.parse::<u32>().ok().map(|h| match meridiem {
                Some(true) if h < 12 => h + 12,
                Some(false) if h == 12 => 0,
                // "at 3" means the afternoon; nobody sets a 3 AM reminder by voice.
                None if (1..=7).contains(&h) => h + 12,
                None if day_word.as_deref() == Some("tonight") && h < 12 => h + 12,
                _ => h,
            }),
        };
        if let Some(hour) = hour.filter(|h| *h < 24 && minute < 60) {
            let today = now.date_naive();
            let day = match day_word.as_deref() {
                Some("tomorrow") => today.succ_opt().unwrap_or(today),
                _ => today,
            };
            if let Some(mut at) = local_at(day, hour, minute) {
                if at <= now && day_word.is_none() {
                    at += Duration::days(1);
                }
                return (Some(at), cut(c.get(0).unwrap()));
            }
        }
    }

    if let Some(c) = DAY_ONLY.captures(text) {
        let tomorrow = c.get(1).unwrap().as_str().eq_ignore_ascii_case("tomorrow");
        let hour = match (tomorrow, c.get(2).map(|p| p.as_str().to_lowercase())) {
            (_, Some(p)) if p == "afternoon" => 14,
            (_, Some(p)) if p == "evening" => 18,
            (false, _) => 19,
            _ => 9,
        };
        let today = now.date_naive();
        let day = if tomorrow { today.succ_opt().unwrap_or(today) } else { today };
        if let Some(at) = local_at(day, hour, 0).filter(|at| *at > now) {
            return (Some(at), cut(c.get(0).unwrap()));
        }
    }

    (None, text.to_string())
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

const ALIASES: &[(&str, &str)] = &[
    ("claude", "claude"),
    ("chatgpt", "chatgpt"),
    ("gpt", "chatgpt"),
    ("openai", "chatgpt"),
    ("grok", "grok"),
    ("cursor", "cursor"),
    ("copilot", "copilot"),
    ("local", "local"),
    ("ollama", "local"),
];

/// Matches the start of a command against agent names, then provider names,
/// like the dashboard's ask box does with "grok: ..." and "Daily Planner, ...".
fn route(text: &str, agents: &[(i64, String)]) -> Option<(String, String, String)> {
    let ws = words(text);
    let rest_after = |n: usize| -> String {
        let from = ws.get(n).map_or(text.len(), |(s, _)| *s);
        let rest = tidy(&text[from..]);
        rest.strip_prefix("to ").map(str::to_string).unwrap_or(rest)
    };
    let mut best: Option<(usize, i64, &String)> = None;
    for (id, name) in agents {
        let n = words(name).len();
        if n == 0 || n > ws.len() {
            continue;
        }
        let spoken: String = ws[..n].iter().map(|(_, w)| w.as_str()).collect();
        if spoken == norm(name) && best.is_none_or(|(len, _, _)| n > len) {
            best = Some((n, *id, name));
        }
    }
    if let Some((n, id, name)) = best {
        return Some((format!("agent:{id}"), name.clone(), rest_after(n)));
    }
    let first = ws.first()?;
    let (_, provider) = ALIASES.iter().find(|(alias, _)| *alias == first.1)?;
    let name = providers::find(provider).map_or(*provider, |p| p.name);
    Some((format!("provider:{provider}"), name.to_string(), rest_after(1)))
}

/// Decides what a spoken command is. Anything that isn't clearly a reminder
/// or an agent request becomes a todo, the same as Enter in the command bar.
pub fn parse(command: &str, agents: &[(i64, String)], now: DateTime<Local>) -> Parsed {
    let text = tidy(command);
    let todo = |title: &str, sure: bool| Parsed {
        kind: "todo".into(),
        title: capitalize(&tidy(title)),
        due_at: None,
        target: String::new(),
        target_name: String::new(),
        sure,
    };

    if let Some(m) = REMIND.find(&text) {
        let (at, rest) = when(&text[m.end()..], now);
        let rest = tidy(&rest);
        let rest = rest.strip_prefix("to ").unwrap_or(&rest);
        return Parsed {
            kind: "reminder".into(),
            title: capitalize(tidy(rest).as_str()),
            due_at: at.map(|a| a.with_timezone(&Utc).to_rfc3339()),
            target: String::new(),
            target_name: String::new(),
            sure: at.is_some(),
        };
    }
    if let Some(c) = ADD_TO_LIST.captures(&text) {
        return todo(c.get(1).unwrap().as_str(), true);
    }
    if let Some(m) = TODO.find(&text) {
        if m.end() < text.len() {
            return todo(&text[m.end()..], true);
        }
    }
    let asked = ASK.find(&text).map_or(text.as_str(), |m| &text[m.end()..]);
    if let Some((target, target_name, rest)) = route(asked, agents) {
        if !rest.is_empty() {
            return Parsed { kind: "agent".into(), title: capitalize(&rest), due_at: None, target, target_name, sure: true };
        }
    }
    todo(&text, false)
}

// ---------- The local model, for commands the rules can't place ----------

fn ai_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": ["todo", "reminder", "agent"] },
            "title": { "type": "string" },
            "remind_at": { "type": "string" },
            "agent": { "type": "string" }
        },
        "required": ["kind", "title", "remind_at", "agent"]
    })
}

fn ai_prompt(agents: &[(i64, String)], now: DateTime<Local>) -> String {
    let mut names: Vec<String> = agents.iter().map(|(_, n)| n.clone()).collect();
    names.extend(["Claude", "ChatGPT", "Grok", "Cursor", "Local AI"].map(String::from));
    format!(
        "You read one spoken command James gave his personal command center app and say what it should do. \
         Reply with JSON only.\n\
         - kind \"reminder\": he wants to be alerted at a time. remind_at is that local time as YYYY-MM-DDTHH:MM.\n\
         - kind \"agent\": he is asking one of these AI agents to do something: {}. agent is that name.\n\
         - kind \"todo\": anything else he needs to do.\n\
         title is a short task in his words, without the time or the agent's name, starting with a capital letter. \
         Leave remind_at and agent empty when they don't apply.\n\
         It is now {} ({}).",
        names.join(", "),
        now.format("%Y-%m-%dT%H:%M"),
        now.format("%A"),
    )
}

/// Turns the model's answer into a command, checking everything it says.
/// None if the answer is unusable, so the rules' guess stands.
fn from_ai(v: &Value, agents: &[(i64, String)], now: DateTime<Local>) -> Option<Parsed> {
    let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let title = capitalize(&tidy(field("title")));
    if title.is_empty() {
        return None;
    }
    let base = Parsed { kind: "todo".into(), title, due_at: None, target: String::new(), target_name: String::new(), sure: true };
    match field("kind") {
        "reminder" => {
            let at = chrono::NaiveDateTime::parse_from_str(field("remind_at"), "%Y-%m-%dT%H:%M")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(field("remind_at"), "%Y-%m-%dT%H:%M:%S"))
                .ok()
                .and_then(|t| Local.from_local_datetime(&t).earliest())
                .filter(|t| *t > now);
            Some(Parsed { kind: "reminder".into(), due_at: at.map(|a| a.with_timezone(&Utc).to_rfc3339()), sure: at.is_some(), ..base })
        }
        "agent" => match route(field("agent"), agents) {
            Some((target, target_name, _)) => Some(Parsed { kind: "agent".into(), target, target_name, ..base }),
            None => Some(base),
        },
        _ => Some(base),
    }
}

async fn interpret(heard: &str, agents: &[(i64, String)], now: DateTime<Local>, model: &str) -> Result<Option<Parsed>, String> {
    let v = ollama::complete_json(
        model,
        &ai_prompt(agents, now),
        heard,
        ai_schema(),
        std::time::Duration::from_secs(AI_TIMEOUT_SECS),
    )
    .await?;
    Ok(from_ai(&v, agents, now))
}

// ---------- Acting on commands ----------

/// Stores the commands found in one recording and runs the ones the
/// auto-run setting allows.
async fn ingest(core: &Arc<Core>, recording_id: &str, recorded_at: Option<String>, transcript: &str) -> Result<(), String> {
    let (wake, auto, ai, ai_model, agents) = {
        let conn = core.db.0.lock().unwrap();
        let agents: Vec<(i64, String)> =
            db::list_agents(&conn).map_err(|e| e.to_string())?.into_iter().map(|a| (a.id, a.name)).collect();
        (
            setting(&conn, SET_WAKE).unwrap_or_else(|| DEFAULT_WAKE.into()),
            Auto::parse(setting(&conn, SET_AUTO)),
            ai_on(&conn),
            setting(&conn, SET_AI_MODEL).unwrap_or_default(),
            agents,
        )
    };
    let found = extract(transcript, &wake);
    if found.is_empty() {
        return Ok(());
    }
    let now = Local::now();
    let mut read = vec![];
    for heard in &found {
        let p = parse(heard, &agents, now);
        if p.sure || !ai {
            read.push((p, "rules"));
            continue;
        }
        match interpret(heard, &agents, now, &ai_model).await {
            Ok(Some(better)) => read.push((better, "local")),
            Ok(None) => read.push((p, "rules")),
            Err(e) => {
                eprintln!("pocket: local model unavailable, using the rules: {e}");
                read.push((p, "rules"));
            }
        }
    }
    let mut to_run = vec![];
    {
        let conn = core.db.0.lock().unwrap();
        for (seq, (heard, (mut p, via))) in found.iter().zip(read).enumerate() {
            // "Remind me to ..." with no time heard goes on the todo list rather
            // than waiting in the Pocket widget for someone to pick a time.
            if p.kind == "reminder" && p.due_at.is_none() && auto != Auto::None {
                p.kind = "todo".into();
            }
            conn.execute(
                "INSERT OR IGNORE INTO pocket_commands (recording_id, seq, recorded_at, heard, kind, title, due_at, target, target_name, status, created_at, via)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11)",
                params![recording_id, seq as i64, recorded_at, heard, p.kind, p.title, p.due_at, p.target, p.target_name, db::now(), via],
            )
            .map_err(|e| e.to_string())?;
            let id = conn.last_insert_rowid();
            if auto.runs(&p) {
                to_run.push(id);
            } else {
                let why = if p.kind == "reminder" && p.due_at.is_none() { " (needs a time)" } else { "" };
                core.notify("Pocket command to review", format!("{}{why}", describe(&p.kind, &p.title, &p.target_name)));
            }
        }
    }
    core.changed("pocket-changed");
    for id in to_run {
        match run(core, id, None).await {
            Ok(c) if c.status == "done" => core.notify("Pocket", c.result),
            Ok(c) => core.notify("Pocket command failed", c.result),
            Err(e) => core.notify("Pocket command failed", e),
        }
    }
    Ok(())
}

fn describe(kind: &str, title: &str, target_name: &str) -> String {
    match kind {
        "reminder" => format!("Reminder: {title}"),
        "agent" => format!("Ask {target_name}: {title}"),
        _ => format!("Todo: {title}"),
    }
}

/// Changes made before approving a command.
#[derive(Deserialize, Debug, Default)]
pub struct Edit {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub due_at: Option<String>,
}

/// Carries out one command, with any edits, and records how it went.
pub async fn run(core: &Arc<Core>, id: i64, edit: Option<Edit>) -> Result<Command, String> {
    let mut c = get(&core.db.0.lock().unwrap(), id).map_err(|e| e.to_string())?.ok_or("That command no longer exists.")?;
    if c.status == "done" {
        return Err("That command already ran.".into());
    }
    if let Some(e) = edit {
        if let Some(k) = e.kind.filter(|k| ["todo", "reminder", "agent"].contains(&k.as_str())) {
            c.kind = k;
        }
        if let Some(t) = e.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            c.title = t;
        }
        if e.due_at.is_some() {
            c.due_at = e.due_at.filter(|d| !d.trim().is_empty());
        }
        core.db
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE pocket_commands SET kind = ?2, title = ?3, due_at = ?4 WHERE id = ?1",
                params![id, c.kind, c.title, c.due_at],
            )
            .map_err(|e| e.to_string())?;
    }

    let outcome: Result<String, String> = match c.kind.as_str() {
        "reminder" => match &c.due_at {
            None => Err("Pick a time for this reminder first.".into()),
            Some(at) => {
                let r = db::add_reminder(
                    &core.db.0.lock().unwrap(),
                    db::NewReminder { title: c.title.clone(), remind_at: at.clone(), repeat: "none".into(), project_id: None },
                );
                r.map(|r| {
                    core.changed("reminders-changed");
                    let at = DateTime::parse_from_rfc3339(&r.remind_at).map(|d| d.with_timezone(&Local).format("%a %-I:%M %p").to_string());
                    format!("Reminder set for {}: {}", at.unwrap_or(r.remind_at), r.title)
                })
            }
        },
        "agent" => {
            let ran = match c.target.split_once(':') {
                Some(("agent", id)) => match id.parse() {
                    Ok(id) => commands::run_agent(core, id, c.title.clone()).await,
                    Err(_) => Err("That agent no longer exists.".into()),
                },
                Some(("provider", p)) => commands::ask_provider(core, p, c.title.clone()).await,
                _ => Err("Say which agent should handle this, such as \"Cursor, ...\".".into()),
            };
            ran.map(|_| format!("Sent to {}: {}", c.target_name, c.title))
        }
        _ => {
            let when = c.recorded_at.as_deref().and_then(|d| DateTime::parse_from_rfc3339(d).ok());
            let notes = match when {
                Some(d) => format!("From Pocket, {}: “{}”", d.with_timezone(&Local).format("%a %b %-d %-I:%M %p"), c.heard),
                None => format!("From Pocket: “{}”", c.heard),
            };
            db::add_todo(
                &core.db.0.lock().unwrap(),
                db::NewTodo { title: c.title.clone(), notes, priority: 2, due_at: None, project_id: None },
            )
            .map(|t| format!("Todo added: {}", t.title))
            .map_err(|e| e.to_string())
        }
    };

    let (status, result) = match outcome {
        Ok(r) => ("done", r),
        Err(e) => ("error", e),
    };
    let conn = core.db.0.lock().unwrap();
    finish(&conn, id, status, &result).map_err(|e| e.to_string())?;
    let c = get(&conn, id).map_err(|e| e.to_string())?.ok_or("That command no longer exists.")?;
    drop(conn);
    core.changed("pocket-changed");
    Ok(c)
}

pub fn dismiss(core: &Core, id: i64) -> Result<(), String> {
    let conn = core.db.0.lock().unwrap();
    conn.execute("UPDATE pocket_commands SET status = 'dismissed' WHERE id = ?1 AND status != 'done'", [id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    core.changed("pocket-changed");
    Ok(())
}

// ---------- Polling ----------

#[derive(Deserialize)]
struct ListPage {
    #[serde(default)]
    data: Vec<Value>,
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(default)]
    has_more: bool,
}

/// Joins transcript segments into one string. The API returns
/// `{ "segments": [...] }` and the webhook a bare array; both are accepted.
fn transcript_text(v: &Value) -> String {
    let segments = v.get("segments").unwrap_or(v);
    segments
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn parse_time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    v.and_then(Value::as_str).and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

async fn get_json(client: &reqwest::Client, key: &str, url: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let res = client
        .get(url)
        .bearer_auth(key)
        .header("Accept", "application/json")
        .query(query)
        .send()
        .await
        .map_err(|e| format!("Could not reach Pocket: {e}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Pocket rejected the API key. Make a new one in the Pocket app under Settings, Developer, API Keys.".into());
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err("Pocket refused the request. Your Pocket plan may not include API access.".into());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("Pocket is rate limiting requests. Will retry shortly.".into());
    }
    if !status.is_success() {
        return Err(format!("Pocket returned {status}."));
    }
    res.json().await.map_err(|e| format!("Unexpected response from Pocket: {e}"))
}

/// Reads recordings made since connecting that haven't been read yet.
async fn poll(core: &Arc<Core>, key: &str) -> Result<(), String> {
    let since = {
        let conn = core.db.0.lock().unwrap();
        prune_seen(&conn).map_err(|e| e.to_string())?;
        setting(&conn, SET_SINCE).and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc))
    };
    let since = since.unwrap_or_else(Utc::now);
    let client = reqwest::Client::new();
    let mut fresh = vec![];
    for page in 1..=MAX_PAGES {
        let query = [
            ("start_date", since.format("%Y-%m-%d").to_string()),
            ("page", page.to_string()),
            ("limit", PAGE_SIZE.to_string()),
        ];
        let body: ListPage = serde_json::from_value(get_json(&client, key, API_URL, &query).await?)
            .map_err(|e| format!("Unexpected response from Pocket: {e}"))?;
        for r in &body.data {
            let Some(id) = r.get("id").and_then(Value::as_str) else { continue };
            let at = parse_time(r.get("recording_at")).or_else(|| parse_time(r.get("created_at")));
            if at.is_some_and(|at| at < since) || is_seen(&core.db.0.lock().unwrap(), id).map_err(|e| e.to_string())? {
                continue;
            }
            fresh.push((id.to_string(), at));
        }
        if !body.pagination.is_some_and(|p| p.has_more) {
            break;
        }
    }

    for (id, at) in fresh {
        let url = format!("{API_URL}/{id}");
        let detail = get_json(&client, key, &url, &[("include_summarizations", "false".into())]).await?;
        let data = detail.get("data").unwrap_or(&detail);
        let text = data.get("transcript").map(transcript_text).unwrap_or_default();
        if text.trim().is_empty() {
            // Still being transcribed; look again next time, but not forever.
            let old = at.is_some_and(|at| Utc::now() - at > Duration::hours(TRANSCRIPT_WAIT_HOURS));
            if old {
                claim(&core.db.0.lock().unwrap(), &id).map_err(|e| e.to_string())?;
            }
            continue;
        }
        if claim(&core.db.0.lock().unwrap(), &id).map_err(|e| e.to_string())? {
            ingest(core, &id, at.map(|a| a.to_rfc3339()), &text).await?;
        }
    }
    Ok(())
}

// ---------- Webhook ----------

/// HMAC-SHA256 (RFC 2104), so no extra crate is needed for one signature check.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks Pocket's `X-HeyPocket-Signature` (hex HMAC-SHA256 of
/// "{timestamp}.{raw body}") and that `X-HeyPocket-Timestamp` (milliseconds) is recent.
pub fn verify(secret: &str, timestamp: &str, signature: &str, body: &[u8], now_ms: i64) -> bool {
    let Ok(ts) = timestamp.trim().parse::<i64>() else { return false };
    if (now_ms - ts).abs() > WEBHOOK_MAX_AGE_MS {
        return false;
    }
    let mut msg = Vec::with_capacity(timestamp.len() + 1 + body.len());
    msg.extend_from_slice(timestamp.trim().as_bytes());
    msg.push(b'.');
    msg.extend_from_slice(body);
    let expected = hex(&hmac_sha256(secret.as_bytes(), &msg));
    let given = signature.trim();
    let given = given.strip_prefix("sha256=").unwrap_or(given).to_ascii_lowercase();
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub enum HookError {
    /// No webhook secret is saved, so the hook is off.
    Disabled,
    BadSignature,
    BadBody(String),
}

/// Handles one webhook delivery. Returns once the request is verified and
/// parsed; the commands run in the background so Pocket isn't kept waiting.
pub fn webhook(core: &Arc<Core>, timestamp: &str, signature: &str, body: &[u8]) -> Result<(), HookError> {
    let secret = core.secrets.get(POCKET_WEBHOOK_SECRET).ok().flatten().ok_or(HookError::Disabled)?;
    if !verify(&secret, timestamp, signature, body, Utc::now().timestamp_millis()) {
        return Err(HookError::BadSignature);
    }
    let v: Value = serde_json::from_slice(body).map_err(|e| HookError::BadBody(e.to_string()))?;
    let Some(recording) = v.get("recording") else { return Ok(()) };
    let Some(id) = recording.get("id").and_then(Value::as_str).map(str::to_string) else { return Ok(()) };
    let text = v.get("transcript").map(transcript_text).unwrap_or_default();
    if text.trim().is_empty() {
        // Not transcribed yet (or a deletion); a later event or the poll picks it up.
        return Ok(());
    }
    let at = parse_time(recording.get("recordingAt")).or_else(|| parse_time(recording.get("createdAt")));
    let core = core.clone();
    tokio::spawn(async move {
        let since = setting(&core.db.0.lock().unwrap(), SET_SINCE).and_then(|s| DateTime::parse_from_rfc3339(&s).ok());
        if at.zip(since).is_some_and(|(at, since)| at < since) {
            return;
        }
        let claimed = claim(&core.db.0.lock().unwrap(), &id).unwrap_or(false);
        if claimed {
            if let Err(e) = ingest(&core, &id, at.map(|a| a.to_rfc3339()), &text).await {
                eprintln!("pocket webhook failed: {e}");
            }
        }
    });
    Ok(())
}

// ---------- State ----------

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub has_key: bool,
    pub has_webhook_secret: bool,
    pub wake: String,
    pub auto: Auto,
    /// Whether the local model reads commands the rules can't place.
    pub ai: bool,
    /// Which local model; empty means the smallest one pulled.
    pub ai_model: String,
    /// Recordings made before this are ignored.
    pub since: Option<String>,
    pub checked_at: Option<String>,
    pub error: Option<String>,
    pub commands: Vec<Command>,
}

#[derive(Default)]
pub struct PocketState {
    inner: Mutex<(Option<String>, Option<String>)>,
}

impl PocketState {
    pub fn snapshot(&self, core: &Core) -> Result<Snapshot, String> {
        let has_key = core.secrets.get(POCKET_KEY)?.is_some();
        let has_webhook_secret = core.secrets.get(POCKET_WEBHOOK_SECRET)?.is_some();
        let conn = core.db.0.lock().unwrap();
        let (checked_at, error) = self.inner.lock().unwrap().clone();
        Ok(Snapshot {
            has_key,
            has_webhook_secret,
            wake: setting(&conn, SET_WAKE).unwrap_or_else(|| DEFAULT_WAKE.into()),
            auto: Auto::parse(setting(&conn, SET_AUTO)),
            ai: ai_on(&conn),
            ai_model: setting(&conn, SET_AI_MODEL).unwrap_or_default(),
            since: setting(&conn, SET_SINCE),
            checked_at: if has_key { checked_at } else { None },
            error: if has_key { error } else { None },
            commands: list(&conn, 50).map_err(|e| e.to_string())?,
        })
    }
}

/// Checks Pocket now and returns the latest state.
pub async fn refresh(core: &Arc<Core>) -> Result<Snapshot, String> {
    if let Some(key) = core.secrets.get(POCKET_KEY)? {
        let result = poll(core, &key).await;
        *core.pocket.inner.lock().unwrap() = (Some(Utc::now().to_rfc3339()), result.err());
        core.changed("pocket-changed");
    }
    core.pocket.snapshot(core)
}

pub async fn poll_loop(core: Arc<Core>) {
    loop {
        if let Err(e) = refresh(&core).await {
            eprintln!("pocket poll failed: {e}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        Local.from_local_datetime(&chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap().and_hms_opt(h, m, 0).unwrap()).unwrap()
    }

    fn local(s: &Option<String>) -> String {
        DateTime::parse_from_rfc3339(s.as_ref().unwrap()).unwrap().with_timezone(&Local).format("%d %H:%M").to_string()
    }

    #[test]
    fn finds_commands_only_after_the_wake_phrase() {
        assert_eq!(extract("Command Center, add a todo to renew the domain.", DEFAULT_WAKE), vec!["add a todo to renew the domain"]);
        assert_eq!(extract("Okay, command centre remind me at 3 PM to call Dana.", DEFAULT_WAKE), vec!["remind me at 3 PM to call Dana"]);
        assert_eq!(
            extract("Command center, todo order cables. Command center, Cursor fix the login bug.", DEFAULT_WAKE),
            vec!["todo order cables", "Cursor fix the login bug"]
        );
        assert_eq!(extract("Commandcenter: todo test", DEFAULT_WAKE), vec!["todo test"]);
        // A meeting that mentions the app later on is not a command.
        assert!(extract("So we talked about the budget and the command center rollout.", DEFAULT_WAKE).is_empty());
        assert!(extract("Command center.", DEFAULT_WAKE).is_empty());
        assert_eq!(extract("Jarvis, todo buy milk", "Jarvis"), vec!["todo buy milk"]);
    }

    #[test]
    fn parses_todos() {
        let now = at(10, 0);
        let p = parse("add a todo to renew the domain", &[], now);
        assert_eq!((p.kind.as_str(), p.title.as_str()), ("todo", "Renew the domain"));
        assert_eq!(parse("To-do: order cables", &[], now).title, "Order cables");
        assert_eq!(parse("add patch the ESXi host to my to do list", &[], now).title, "Patch the ESXi host");
        // Anything unclear becomes a todo.
        let p = parse("pick up the new switch from the office", &[], now);
        assert_eq!((p.kind.as_str(), p.title.as_str()), ("todo", "Pick up the new switch from the office"));
    }

    #[test]
    fn parses_reminder_times() {
        let now = at(10, 0);
        let p = parse("remind me at 3 PM to call Dana", &[], now);
        assert_eq!((p.kind.as_str(), p.title.as_str()), ("reminder", "Call Dana"));
        assert_eq!(local(&p.due_at), "20 15:00");
        assert_eq!(local(&parse("remind me to call Dana at 3:30 p.m.", &[], now).due_at), "20 15:30");
        assert_eq!(local(&parse("remind me to check backups in 20 minutes", &[], now).due_at), "20 10:20");
        assert_eq!(local(&parse("remind me in an hour to stretch", &[], now).due_at), "20 11:00");
        assert_eq!(local(&parse("remind me in half an hour to stretch", &[], now).due_at), "20 10:30");
        assert_eq!(local(&parse("remind me tomorrow at 8 am to send the invoice", &[], now).due_at), "21 08:00");
        assert_eq!(local(&parse("remind me to send the invoice tomorrow", &[], now).due_at), "21 09:00");
        // "at 9" when it's already 10 means tomorrow morning.
        assert_eq!(local(&parse("remind me at 9 to check the VM", &[], now).due_at), "21 09:00");
        assert_eq!(parse("remind me to send the invoice tomorrow", &[], now).title, "Send the invoice");
        let p = parse("set a reminder to call the ISP", &[], now);
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.due_at.is_none()), ("reminder", "Call the ISP", true));
    }

    #[test]
    fn routes_agent_requests() {
        let now = at(10, 0);
        let agents = vec![(1, "Daily Planner".to_string()), (2, "Bugfixer".to_string())];
        let p = parse("Cursor, fix the login bug", &agents, now);
        assert_eq!((p.kind.as_str(), p.target.as_str(), p.title.as_str()), ("agent", "provider:cursor", "Fix the login bug"));
        let p = parse("ask Daily Planner what's left for today", &agents, now);
        assert_eq!((p.target.as_str(), p.target_name.as_str(), p.title.as_str()), ("agent:1", "Daily Planner", "What's left for today"));
        assert_eq!(parse("tell bugfixer to update the readme", &agents, now).title, "Update the readme");
        // A name alone isn't a request.
        assert_eq!(parse("Grok.", &agents, now).kind, "todo");
    }

    #[test]
    fn auto_run_policy() {
        let p = |kind: &str, due: bool| Parsed {
            kind: kind.into(),
            title: "x".into(),
            due_at: due.then(|| "2026-10-20T15:00:00Z".into()),
            target: String::new(),
            target_name: String::new(),
            sure: true,
        };
        assert!(Auto::Tasks.runs(&p("todo", false)));
        assert!(Auto::Tasks.runs(&p("reminder", true)));
        assert!(!Auto::Tasks.runs(&p("reminder", false)));
        assert!(!Auto::Tasks.runs(&p("agent", false)));
        assert!(Auto::All.runs(&p("agent", false)));
        assert!(!Auto::None.runs(&p("todo", false)));
    }

    #[test]
    fn rules_say_when_they_are_only_guessing() {
        let now = at(10, 0);
        assert!(parse("add a todo to renew the domain", &[], now).sure);
        assert!(parse("remind me at 3 PM to call Dana", &[], now).sure);
        assert!(!parse("remind me to call Dana after lunch", &[], now).sure);
        assert!(!parse("I need to get the Ubiquiti quote over to Mike before the end of the week", &[], now).sure);
    }

    #[test]
    fn checks_what_the_local_model_says() {
        let now = at(10, 0);
        let agents = vec![(1, "Daily Planner".to_string())];
        let v = serde_json::json!({ "kind": "reminder", "title": "call Dana", "remind_at": "2026-10-20T13:00", "agent": "" });
        let p = from_ai(&v, &agents, now).unwrap();
        assert_eq!((p.kind.as_str(), p.title.as_str(), local(&p.due_at).as_str(), p.sure), ("reminder", "Call Dana", "20 13:00", true));
        // A time in the past isn't trusted; the reminder waits for one.
        let v = serde_json::json!({ "kind": "reminder", "title": "x", "remind_at": "2026-10-19T13:00", "agent": "" });
        assert!(from_ai(&v, &agents, now).unwrap().due_at.is_none());
        let v = serde_json::json!({ "kind": "agent", "title": "Plan my afternoon", "remind_at": "", "agent": "daily planner" });
        assert_eq!(from_ai(&v, &agents, now).unwrap().target, "agent:1");
        let v = serde_json::json!({ "kind": "agent", "title": "Fix the bug", "remind_at": "", "agent": "Local AI" });
        assert_eq!(from_ai(&v, &agents, now).unwrap().target, "provider:local");
        // An agent that doesn't exist becomes a todo; an empty title is no answer at all.
        let v = serde_json::json!({ "kind": "agent", "title": "Do it", "remind_at": "", "agent": "Jarvis" });
        assert_eq!(from_ai(&v, &agents, now).unwrap().kind, "todo");
        assert!(from_ai(&serde_json::json!({ "kind": "todo", "title": " " }), &agents, now).is_none());
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn verifies_pocket_signatures() {
        let body = br#"{"event":"summary.completed"}"#;
        let ts = "1760000000000";
        let sig = hex(&hmac_sha256(b"whsec_test", format!("{ts}.{}", std::str::from_utf8(body).unwrap()).as_bytes()));
        let now = 1_760_000_000_000 + 1_000;
        assert!(verify("whsec_test", ts, &sig, body, now));
        assert!(verify("whsec_test", ts, &format!("sha256={}", sig.to_uppercase()), body, now));
        assert!(!verify("other", ts, &sig, body, now));
        assert!(!verify("whsec_test", ts, &sig, br#"{"event":"x"}"#, now));
        // Too old to trust.
        assert!(!verify("whsec_test", ts, &sig, body, now + WEBHOOK_MAX_AGE_MS));
    }

    #[test]
    fn reads_both_transcript_shapes() {
        let api = serde_json::json!({ "segments": [{ "text": "Command center," }, { "text": "todo test" }] });
        let hook = serde_json::json!([{ "text": "Command center," }, { "text": "todo test" }]);
        assert_eq!(transcript_text(&api), "Command center, todo test");
        assert_eq!(transcript_text(&hook), transcript_text(&api));
    }
}
