import { useState } from "react";
import { api } from "../api";
import { useData } from "../data";
import { minutesFromNow, quick, tomorrowAt9 } from "../quick";
import { formatWhen } from "../time";
import Widget from "./Widget";

const DAY = 86400_000;

export default function RemindersWidget() {
  const { reminders, act } = useData();
  const [title, setTitle] = useState("");
  const t = title.trim();

  // Reminders that went off in the last day stay visible so they can be snoozed.
  const justFired = reminders.filter((r) => r.fired && Date.now() - new Date(r.remind_at).getTime() < DAY);
  const upcoming = reminders.filter((r) => !r.fired);

  async function add(at: Date) {
    if (!t) return;
    const saved = await act(() => quick.remind(t, at));
    if (saved) setTitle("");
  }

  return (
    <Widget title="Reminders">
      <form
        className="inline-add"
        onSubmit={(e) => {
          e.preventDefault();
          add(minutesFromNow(60));
        }}
      >
        <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Remind me to…" aria-label="New reminder" />
        <div className="chips">
          <button type="button" disabled={!t} onClick={() => add(minutesFromNow(15))}>15 min</button>
          <button type="submit" disabled={!t}>1 hour</button>
          <button type="button" disabled={!t} onClick={() => add(tomorrowAt9())}>Tomorrow 9 AM</button>
        </div>
      </form>

      {justFired.length > 0 && (
        <ul className="list dense">
          {justFired.map((r) => (
            <li key={r.id} className="fired">
              <span className="status-dot warning" aria-hidden="true" />
              <span className="grow truncate" title={r.title}>
                {r.title}
                <span className="sub">Went off {formatWhen(r.remind_at)}</span>
              </span>
              <button onClick={() => act(() => api.snoozeReminder(r.id, 15))}>Snooze 15</button>
              <button className="ghost row-action" onClick={() => act(() => api.deleteReminder(r.id))} aria-label={`Dismiss ${r.title}`}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}

      {upcoming.length === 0 && justFired.length === 0 ? (
        <p className="empty">No reminders scheduled.</p>
      ) : (
        <ul className="list dense">
          {upcoming.map((r) => (
            <li key={r.id}>
              <span className="grow truncate" title={r.title}>{r.title}</span>
              {r.repeat !== "none" && <span className="tag">{r.repeat}</span>}
              <span className="tag">{formatWhen(r.remind_at)}</span>
              <button className="ghost row-action" onClick={() => act(() => api.deleteReminder(r.id))} aria-label={`Delete ${r.title}`}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </Widget>
  );
}
