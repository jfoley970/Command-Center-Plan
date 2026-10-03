//! The pomodoro timer in the top margin. The backend owns it so every open
//! window and browser tab shows the same countdown, it keeps running when the
//! app is closed to the tray, and the "time's up" notification comes from the
//! same place as reminder notifications. State is saved in the settings table
//! so a restart doesn't lose a running session.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::{db, Core};

const SETTING: &str = "pomodoro";
pub const WORK_MINUTES: i64 = 25;
pub const BREAK_MINUTES: i64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Work,
    Break,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finished {
    pub phase: Phase,
    pub at: DateTime<Utc>,
}

/// What is saved. Idle when neither `ends_at` nor `paused_ms` is set.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Timer {
    phase: Phase,
    minutes: i64,
    ends_at: Option<DateTime<Utc>>,
    paused_ms: Option<i64>,
    /// The session that just ran out, until someone answers the popup.
    finished: Option<Finished>,
}

impl Default for Timer {
    fn default() -> Self {
        Self { phase: Phase::Work, minutes: WORK_MINUTES, ends_at: None, paused_ms: None, finished: None }
    }
}

/// What the UI gets. `remaining_ms` rather than an end time, so a browser whose
/// clock differs from the server's still counts down correctly.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub phase: Phase,
    pub minutes: i64,
    pub state: &'static str,
    pub remaining_ms: i64,
    pub finished: Option<Finished>,
}

pub struct PomodoroState(Mutex<Timer>);

impl PomodoroState {
    pub fn load(db: &db::Db) -> Self {
        let saved = db::get_setting(&db.0.lock().unwrap(), SETTING).ok().flatten();
        Self(Mutex::new(saved.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()))
    }
}

fn snapshot(t: &Timer, now: DateTime<Utc>) -> Snapshot {
    let (state, remaining_ms) = match (t.ends_at, t.paused_ms) {
        (Some(end), _) => ("running", (end - now).num_milliseconds().max(0)),
        (None, Some(ms)) => ("paused", ms),
        (None, None) => ("idle", t.minutes * 60_000),
    };
    Snapshot { phase: t.phase, minutes: t.minutes, state, remaining_ms, finished: t.finished.clone() }
}

/// Applies a change, saves it and tells every client.
fn update(core: &Arc<Core>, f: impl FnOnce(&mut Timer, DateTime<Utc>)) -> Snapshot {
    let now = Utc::now();
    let (snap, json) = {
        let mut t = core.pomodoro.0.lock().unwrap();
        f(&mut t, now);
        (snapshot(&t, now), serde_json::to_string(&*t).unwrap_or_default())
    };
    if let Err(e) = db::set_setting(&core.db.0.lock().unwrap(), SETTING, &json) {
        eprintln!("could not save the pomodoro timer: {e}");
    }
    core.emit("pomodoro-changed", &snap);
    snap
}

pub fn get(core: &Arc<Core>) -> Snapshot {
    snapshot(&core.pomodoro.0.lock().unwrap(), Utc::now())
}

/// Starts a fresh session. Minutes default to 25 for work and 5 for a break.
pub fn start(core: &Arc<Core>, phase: Phase, minutes: Option<i64>) -> Snapshot {
    let minutes = minutes.unwrap_or(match phase {
        Phase::Work => WORK_MINUTES,
        Phase::Break => BREAK_MINUTES,
    });
    let minutes = minutes.clamp(1, 180);
    update(core, |t, now| {
        *t = Timer { phase, minutes, ends_at: Some(now + Duration::minutes(minutes)), paused_ms: None, finished: None };
    })
}

pub fn pause(core: &Arc<Core>) -> Snapshot {
    update(core, |t, now| {
        if let Some(end) = t.ends_at.take() {
            t.paused_ms = Some((end - now).num_milliseconds().max(0));
        }
    })
}

/// Resumes a paused session, or starts the current phase from the top when idle.
pub fn resume(core: &Arc<Core>) -> Snapshot {
    update(core, |t, now| {
        if t.ends_at.is_some() {
            return;
        }
        let ms = t.paused_ms.take().unwrap_or(t.minutes * 60_000);
        t.ends_at = Some(now + Duration::milliseconds(ms));
        t.finished = None;
    })
}

/// Stops the timer and puts it back to a full work session.
pub fn reset(core: &Arc<Core>) -> Snapshot {
    update(core, |t, _| *t = Timer::default())
}

/// Closes the "time's up" popup everywhere.
pub fn dismiss(core: &Arc<Core>) -> Snapshot {
    update(core, |t, _| t.finished = None)
}

/// Ends a running session once its time is up. Returns the phase that ended.
fn finish_if_due(t: &mut Timer, now: DateTime<Utc>) -> Option<Phase> {
    let end = t.ends_at?;
    if now < end {
        return None;
    }
    let phase = t.phase;
    // Back to a full session of the same kind, ready for the popup's choice.
    *t = Timer { phase, minutes: t.minutes, ends_at: None, paused_ms: None, finished: Some(Finished { phase, at: end }) };
    Some(phase)
}

/// Checks every second whether the running session is up.
pub async fn tick_loop(core: Arc<Core>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let due = core.pomodoro.0.lock().unwrap().ends_at.is_some_and(|end| Utc::now() >= end);
        if !due {
            continue;
        }
        let mut ended = None;
        update(&core, |t, now| ended = finish_if_due(t, now));
        match ended {
            Some(Phase::Work) => core.notify("Pomodoro done", "Nice work. Time for a 5-minute break."),
            Some(Phase::Break) => core.notify("Break's over", "Ready for another 25-minute session?"),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_shows_full_session() {
        let s = snapshot(&Timer::default(), Utc::now());
        assert_eq!((s.state, s.remaining_ms), ("idle", 25 * 60_000));
    }

    #[test]
    fn finishes_only_when_due() {
        let now = Utc::now();
        let mut t = Timer { ends_at: Some(now + Duration::seconds(30)), ..Timer::default() };
        assert_eq!(finish_if_due(&mut t, now), None);
        assert_eq!(snapshot(&t, now).state, "running");

        let later = now + Duration::seconds(31);
        assert_eq!(finish_if_due(&mut t, later), Some(Phase::Work));
        let s = snapshot(&t, later);
        assert_eq!(s.state, "idle");
        assert_eq!(s.finished.map(|f| f.phase), Some(Phase::Work));
        // Already finished; a second check does nothing.
        assert_eq!(finish_if_due(&mut t, later), None);
    }

    #[test]
    fn paused_keeps_remaining_time() {
        let t = Timer { paused_ms: Some(90_000), ..Timer::default() };
        let s = snapshot(&t, Utc::now());
        assert_eq!((s.state, s.remaining_ms), ("paused", 90_000));
    }

    #[test]
    fn saved_state_round_trips() {
        let t = Timer { phase: Phase::Break, minutes: 5, paused_ms: Some(1234), ..Timer::default() };
        let back: Timer = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!((back.phase, back.minutes, back.paused_ms), (Phase::Break, 5, Some(1234)));
    }
}
