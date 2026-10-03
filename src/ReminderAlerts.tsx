// The popup that appears when a reminder goes off, wherever you are in the app.
// It offers to bring the reminder back later, or to mark it done.
import { useEffect, useRef, useState } from "react";
import { api, type Reminder } from "./api";
import { useData } from "./data";
import { listen } from "./transport";
import { formatWhen, fromLocalInput, toLocalInput } from "./time";

const EXTEND = [
  { minutes: 5, label: "5 min" },
  { minutes: 15, label: "15 min" },
  { minutes: 60, label: "1 hour" },
];

export default function ReminderAlerts() {
  const { reminders } = useData();
  // Reminders as they were when they went off.
  const [open, setOpen] = useState<Reminder[]>([]);

  useEffect(() => {
    const off = listen<Reminder[] | null>("reminders-fired", ({ payload }) => {
      // A null payload is a resync, not a reminder going off.
      if (!payload?.length) return;
      setOpen((cur) => [...cur.filter((c) => !payload.some((p) => p.id === c.id)), ...payload]);
    });
    return () => void off.then((f) => f());
  }, []);

  // Drop reminders someone already handled, here or in another window. A one-off
  // was handled once its time moves; a repeating one moves on its own, so only
  // deleting it counts.
  useEffect(() => {
    setOpen((cur) => {
      const next = cur.filter((c) => {
        const now = reminders.find((r) => r.id === c.id);
        return now && (c.repeat !== "none" || now.remind_at === c.remind_at);
      });
      return next.length === cur.length ? cur : next;
    });
  }, [reminders]);

  const close = (id: number) => setOpen((cur) => cur.filter((c) => c.id !== id));

  if (open.length === 0) return null;

  return (
    <div className="modal-overlay" onKeyDown={(e) => e.key === "Escape" && setOpen([])}>
      <div className="modal reminder-alert" role="alertdialog" aria-modal="true" aria-labelledby="reminder-alert-title">
        <header className="modal-head">
          <span className="status-dot warning" aria-hidden="true" />
          <h2 id="reminder-alert-title" className="grow">{open.length === 1 ? "Reminder" : `${open.length} reminders`}</h2>
          <button className="ghost" onClick={() => setOpen([])} title="Close. They stay in the Reminders widget." aria-label="Close">✕</button>
        </header>
        <ul className="alert-list">
          {open.map((r, i) => (
            <AlertItem key={r.id} reminder={r} first={i === 0} onDone={() => close(r.id)} />
          ))}
        </ul>
      </div>
    </div>
  );
}

function AlertItem({ reminder: r, first, onDone }: { reminder: Reminder; first: boolean; onDone: () => void }) {
  const { act } = useData();
  const [custom, setCustom] = useState<string | null>(null);
  const firstBtn = useRef<HTMLButtonElement>(null);
  const repeats = r.repeat !== "none";

  useEffect(() => {
    if (first) firstBtn.current?.focus();
  }, [first]);

  async function run(fn: () => Promise<unknown>) {
    const res = await act(fn);
    if (res !== undefined) onDone();
  }

  function saveCustom() {
    if (!custom) return;
    const at = fromLocalInput(custom);
    // A repeating reminder keeps its schedule; the custom time is a one-off follow-up.
    run(() =>
      repeats
        ? api.addReminder({ title: r.title, remind_at: at, repeat: "none", project_id: r.project_id })
        : api.rescheduleReminder(r.id, at),
    );
  }

  return (
    <li>
      <div className="alert-title">{r.title}</div>
      <div className="sub">
        Due {formatWhen(r.remind_at)}
        {repeats && ` · repeats ${r.repeat}, next one stays on schedule`}
      </div>

      <div className="chips">
        <span className="chips-label">Remind me again in</span>
        {EXTEND.map((x, i) => (
          <button key={x.minutes} ref={i === 0 ? firstBtn : undefined} onClick={() => run(() => api.extendReminder(r.id, x.minutes))}>
            {x.label}
          </button>
        ))}
        <button
          className={custom !== null ? "active" : undefined}
          onClick={() => {
            const d = new Date(Date.now() + 30 * 60 * 1000);
            d.setSeconds(0, 0);
            setCustom(custom === null ? toLocalInput(d) : null);
          }}
          aria-expanded={custom !== null}
        >
          Custom…
        </button>
      </div>

      {custom !== null && (
        <form
          className="time-edit"
          onSubmit={(e) => {
            e.preventDefault();
            saveCustom();
          }}
        >
          <input type="datetime-local" value={custom} onChange={(e) => setCustom(e.target.value)} aria-label="Remind me at" autoFocus required />
          <button type="submit" className="primary" disabled={!custom}>Set</button>
        </form>
      )}

      <div className="alert-actions">
        {repeats ? (
          <button className="primary" onClick={onDone}>Done</button>
        ) : (
          <button className="primary" onClick={() => run(() => api.deleteReminder(r.id).then(() => true))}>Done</button>
        )}
      </div>
    </li>
  );
}
