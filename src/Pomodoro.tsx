// The small pomodoro timer in the top margin, and the popup when a session runs
// out. The backend keeps the time, so every window and browser tab agrees and
// the timer carries on across page switches and with the window closed.
import { useCallback, useEffect, useRef, useState } from "react";
import { useData } from "./data";
import { listen } from "./transport";
import { pomodoro, type Phase, type PomodoroSnapshot } from "./pomodoroApi";

type Timer = PomodoroSnapshot & { /** Local Date.now() when the session ends, if running. */ endsAt: number | null };

const withDeadline = (s: PomodoroSnapshot): Timer => ({ ...s, endsAt: s.state === "running" ? Date.now() + s.remaining_ms : null });

function clock(ms: number): string {
  const total = Math.ceil(ms / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export default function Pomodoro() {
  const { act } = useData();
  const [timer, setTimer] = useState<Timer | null>(null);
  const [now, setNow] = useState(Date.now());

  const load = useCallback(() => {
    pomodoro.get().then((s) => setTimer(withDeadline(s)), () => {});
  }, []);

  useEffect(() => {
    load();
    const off = listen<PomodoroSnapshot | null>("pomodoro-changed", ({ payload }) => {
      // A null payload is a resync after a dropped connection.
      if (payload) setTimer(withDeadline(payload));
      else load();
    });
    return () => void off.then((f) => f());
  }, [load]);

  const running = timer?.state === "running";
  useEffect(() => {
    if (!running) return;
    const id = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(id);
  }, [running]);

  const left = !timer ? 0 : timer.endsAt !== null ? Math.max(0, timer.endsAt - now) : timer.remaining_ms;

  // Show the countdown in the window or tab title while it runs.
  useEffect(() => {
    const base = "Command Center";
    document.title = running && timer ? `${clock(left)} ${timer.phase === "work" ? "focus" : "break"} · ${base}` : base;
  }, [running, left, timer]);
  useEffect(() => () => void (document.title = "Command Center"), []);

  const run = (fn: () => Promise<PomodoroSnapshot>) =>
    act(fn).then((s) => {
      if (s) setTimer(withDeadline(s));
    });

  if (!timer) return null;

  const label = timer.phase === "work" ? "Focus" : "Break";
  return (
    <>
      <div className={`topbar-item pomo pomo-${timer.state} pomo-${timer.phase}`} role="timer" aria-label={`Pomodoro ${label.toLowerCase()}: ${clock(left)} ${timer.state}`}>
        <span className="pomo-dot" aria-hidden="true" />
        <span className="pomo-phase">{label}</span>
        <span className="pomo-time">{clock(left)}</span>
        {running ? (
          <button className="pomo-btn" onClick={() => run(pomodoro.pause)} title="Pause" aria-label="Pause">
            <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><rect x="3.5" y="3" width="3" height="10" rx="0.8" /><rect x="9.5" y="3" width="3" height="10" rx="0.8" /></svg>
          </button>
        ) : (
          <button className="pomo-btn" onClick={() => run(pomodoro.resume)} title={timer.state === "paused" ? "Resume" : `Start ${timer.minutes} min`} aria-label={timer.state === "paused" ? "Resume" : "Start"}>
            <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M4.5 2.8v10.4a.6.6 0 0 0 .9.5l8.2-5.2a.6.6 0 0 0 0-1L5.4 2.3a.6.6 0 0 0-.9.5z" /></svg>
          </button>
        )}
        {(timer.state !== "idle" || timer.phase !== "work") && (
          <button className="pomo-btn" onClick={() => run(pomodoro.reset)} title="Reset to a 25 min session" aria-label="Reset">
            <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true"><path d="M3 8a5 5 0 1 0 1.5-3.6" /><path d="M3 2.5v2.8h2.8" /></svg>
          </button>
        )}
      </div>

      {timer.finished && <PomodoroAlert phase={timer.finished.phase} minutes={timer.minutes} run={run} />}
    </>
  );
}

/** Same look as the reminder popup. */
function PomodoroAlert({ phase, minutes, run }: { phase: Phase; minutes: number; run: (fn: () => Promise<PomodoroSnapshot>) => Promise<void> }) {
  const firstBtn = useRef<HTMLButtonElement>(null);
  useEffect(() => firstBtn.current?.focus(), [phase]);

  const dismiss = () => run(pomodoro.dismiss);
  const work = phase === "work";

  return (
    <div className="modal-overlay" onKeyDown={(e) => e.key === "Escape" && dismiss()}>
      <div className="modal reminder-alert" role="alertdialog" aria-modal="true" aria-labelledby="pomo-alert-title">
        <header className="modal-head">
          <span className={`status-dot ${work ? "warning" : "ok"}`} aria-hidden="true" />
          <h2 id="pomo-alert-title" className="grow">{work ? "Pomodoro done" : "Break's over"}</h2>
          <button className="ghost" onClick={dismiss} title="Dismiss" aria-label="Close">✕</button>
        </header>
        <ul className="alert-list">
          <li>
            <div className="alert-title">{work ? `${minutes} minutes of focus, done.` : "Ready to get back to it?"}</div>
            <div className="sub">{work ? "Take a 5-minute break, or keep going with another session." : "Start another 25-minute session, or take a few more minutes."}</div>
            <div className="alert-actions pomo-actions">
              <button className="ghost" onClick={dismiss}>Dismiss</button>
              {work ? (
                <>
                  <button onClick={() => run(() => pomodoro.start("work"))}>Another 25 min</button>
                  <button ref={firstBtn} className="primary" onClick={() => run(() => pomodoro.start("break"))}>Start 5 min break</button>
                </>
              ) : (
                <>
                  <button onClick={() => run(() => pomodoro.start("break"))}>5 more min</button>
                  <button ref={firstBtn} className="primary" onClick={() => run(() => pomodoro.start("work"))}>Start 25 min session</button>
                </>
              )}
            </div>
          </li>
        </ul>
      </div>
    </div>
  );
}
