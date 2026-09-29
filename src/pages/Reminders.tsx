import { useState, type FormEvent } from "react";
import { api, type Repeat } from "../api";
import { useData } from "../data";
import { fromLocalInput, toLocalInput } from "../time";
import ReminderTime from "../widgets/ReminderTime";

function inAnHour(): string {
  const d = new Date(Date.now() + 60 * 60 * 1000);
  d.setSeconds(0, 0);
  return toLocalInput(d);
}

export default function Reminders() {
  const { reminders, act } = useData();
  const [title, setTitle] = useState("");
  const [when, setWhen] = useState(inAnHour);
  const [repeat, setRepeat] = useState<Repeat>("none");

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!title.trim() || !when) return;
    const saved = await act(() => api.addReminder({ title, remind_at: fromLocalInput(when), repeat }));
    if (saved) {
      setTitle("");
      setWhen(inAnHour());
      setRepeat("none");
    }
  }

  const pending = reminders.filter((r) => !r.fired);
  const past = reminders.filter((r) => r.fired);

  return (
    <div className="page">
      <header className="page-head">
        <h1>Reminders</h1>
        <p className="muted">These fire as desktop notifications, even with the window closed.</p>
      </header>

      <form className="card form-row" onSubmit={add}>
        <input className="grow" placeholder="Remind me to…" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus />
        <input type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} aria-label="When" />
        <select value={repeat} onChange={(e) => setRepeat(e.target.value as Repeat)} aria-label="Repeat">
          <option value="none">Once</option>
          <option value="daily">Daily</option>
          <option value="weekly">Weekly</option>
        </select>
        <button className="primary" type="submit">Add</button>
      </form>

      <section className="card">
        <h2>Upcoming</h2>
        {pending.length === 0 && <p className="muted">Nothing scheduled.</p>}
        <ul className="list">
          {pending.map((r) => (
            <li key={r.id}>
              <span className="grow">{r.title}</span>
              {r.repeat !== "none" && <span className="tag">{r.repeat}</span>}
              <ReminderTime reminder={r} />
              <button className="ghost" onClick={() => act(() => api.deleteReminder(r.id))} aria-label="Delete">✕</button>
            </li>
          ))}
        </ul>
      </section>

      {past.length > 0 && (
        <section className="card">
          <h2>Already fired</h2>
          <ul className="list">
            {past.map((r) => (
              <li key={r.id} className="done">
                <span className="grow">{r.title}</span>
                <ReminderTime reminder={r} />
                <button onClick={() => act(() => api.extendReminder(r.id, 10))}>Snooze 10 min</button>
                <button className="ghost" onClick={() => act(() => api.deleteReminder(r.id))} aria-label="Delete">✕</button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
