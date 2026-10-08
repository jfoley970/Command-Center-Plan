import { useEffect, useState } from "react";
import { api, errorText, type Todo, type TodoEmail } from "../api";
import { useData } from "../data";
import { openUrl } from "../transport";

function fullDate(iso: string): string {
  return new Date(iso).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

/** The expanded part of a todo row: the email it came from, if any, and its notes. */
export default function TodoDetail({ todo, fromEmail }: { todo: Todo; fromEmail: boolean }) {
  const { act } = useData();
  const [email, setEmail] = useState<TodoEmail | null | undefined>(fromEmail ? undefined : null);
  const [emailError, setEmailError] = useState<string | null>(null);
  const [notes, setNotes] = useState(todo.notes);

  useEffect(() => {
    if (!fromEmail) return;
    let live = true;
    api.todoEmail(todo.id).then(
      (e) => live && setEmail(e),
      (e) => live && setEmailError(errorText(e)),
    );
    return () => {
      live = false;
    };
  }, [todo.id, fromEmail]);

  // Pick up edits made elsewhere, unless this box has unsaved typing.
  useEffect(() => setNotes(todo.notes), [todo.notes]);

  function saveNotes() {
    if (notes !== todo.notes) act(() => api.setTodoNotes(todo.id, notes));
  }

  return (
    <div className="todo-detail">
      {fromEmail &&
        (emailError ? (
          <p className="muted small">Couldn't load the email: {emailError}</p>
        ) : email === undefined ? (
          <p className="muted small">Loading email…</p>
        ) : email ? (
          <div className="email-card">
            <div className="row between">
              <strong className="truncate" title={email.subject}>{email.subject || "(no subject)"}</strong>
              {email.web_link && (
                <button className="small-btn" onClick={() => openUrl(email.web_link)} title="Open this email in Outlook">
                  Open in Outlook
                </button>
              )}
            </div>
            <div className="email-meta">
              <span className="truncate" title={email.from_addr}>
                {email.from_name || email.from_addr}
                {email.from_name && email.from_addr && email.from_name !== email.from_addr && <span className="muted"> &lt;{email.from_addr}&gt;</span>}
              </span>
              {email.received_at && <span className="muted">{fullDate(email.received_at)}</span>}
            </div>
            {email.preview ? (
              <p className="email-preview">{email.preview}</p>
            ) : (
              <p className="muted small">The preview shows up after the next mail sync.</p>
            )}
          </div>
        ) : null)}
      <textarea
        className="todo-notes"
        rows={2}
        value={notes}
        placeholder="Add notes…"
        aria-label={`Notes for ${todo.title}`}
        onChange={(e) => setNotes(e.target.value)}
        onBlur={saveNotes}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) (e.target as HTMLTextAreaElement).blur();
          if (e.key === "Escape") setNotes(todo.notes);
        }}
      />
      <p className="muted small">
        Added {fullDate(todo.created_at)}
        {todo.done_at && ` · done ${fullDate(todo.done_at)}`}
      </p>
    </div>
  );
}
