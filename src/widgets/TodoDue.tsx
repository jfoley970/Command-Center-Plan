import { useState } from "react";
import { api, type Todo } from "../api";
import { useData } from "../data";
import { formatWhen, fromLocalInput, isOverdue, nextBusinessClose, toLocalInput } from "../time";

/**
 * A todo's due date as a tag; click it to pick a new date and time in place.
 * A todo without one gets a faint "+ Due" button on hover. Nothing here moves a
 * date on its own: it only changes when a new one is saved.
 */
export default function TodoDue({ todo: t }: { todo: Todo }) {
  const { act } = useData();
  const [value, setValue] = useState<string | null>(null);

  function start() {
    const from = t.due_at ? new Date(t.due_at) : nextBusinessClose();
    from.setSeconds(0, 0);
    setValue(toLocalInput(from));
  }

  async function save(due: string | null) {
    const saved = await act(() => api.setTodoDue(t.id, due).then(() => true));
    if (saved) setValue(null);
  }

  if (value === null) {
    if (!t.due_at) {
      return (
        <button className="ghost row-action due-add" onClick={start} title="Set a due date" aria-label={`Set a due date for ${t.title}`}>
          + Due
        </button>
      );
    }
    const late = isOverdue(t.due_at);
    return (
      <button
        className={late ? "tag tag-btn critical" : "tag tag-btn"}
        onClick={start}
        title="Change due date"
        aria-label={`Due ${formatWhen(t.due_at)}${late ? ", overdue" : ""}. Change due date for ${t.title}`}
      >
        {late && "! "}
        {formatWhen(t.due_at)}
      </button>
    );
  }

  return (
    <form
      className="time-edit"
      onSubmit={(e) => {
        e.preventDefault();
        if (value) save(fromLocalInput(value));
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") setValue(null);
      }}
    >
      <input type="datetime-local" value={value} onChange={(e) => setValue(e.target.value)} aria-label={`Due date for ${t.title}`} autoFocus required />
      <button type="submit" className="primary" disabled={!value}>Save</button>
      {t.due_at && (
        <button type="button" className="ghost" onClick={() => save(null)} title="Remove the due date">
          Clear
        </button>
      )}
      <button type="button" className="ghost" onClick={() => setValue(null)} aria-label="Cancel">✕</button>
    </form>
  );
}
