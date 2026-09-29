import { useState } from "react";
import { api, type Reminder } from "../api";
import { useData } from "../data";
import { formatWhen, fromLocalInput, toLocalInput } from "../time";

/** A reminder's time as a tag; click it to pick a new time in place. */
export default function ReminderTime({ reminder: r }: { reminder: Reminder }) {
  const { act } = useData();
  const [value, setValue] = useState<string | null>(null);

  function start() {
    // A reminder that already went off starts from a sensible future time.
    const at = new Date(r.remind_at);
    const from = at.getTime() > Date.now() ? at : new Date(Date.now() + 60 * 60 * 1000);
    from.setSeconds(0, 0);
    setValue(toLocalInput(from));
  }

  async function save() {
    if (!value) return;
    const saved = await act(() => api.rescheduleReminder(r.id, fromLocalInput(value)));
    if (saved) setValue(null);
  }

  if (value === null) {
    return (
      <button className="tag tag-btn" onClick={start} title="Change time" aria-label={`Change time for ${r.title}`}>
        {formatWhen(r.remind_at)}
      </button>
    );
  }

  return (
    <form
      className="time-edit"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") setValue(null);
      }}
    >
      <input type="datetime-local" value={value} onChange={(e) => setValue(e.target.value)} aria-label={`New time for ${r.title}`} autoFocus required />
      <button type="submit" className="primary" disabled={!value}>Save</button>
      <button type="button" className="ghost" onClick={() => setValue(null)} aria-label="Cancel">✕</button>
    </form>
  );
}
