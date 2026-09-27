import { useState, type FormEvent } from "react";
import { api, type Todo } from "../api";
import { useData } from "../data";
import { formatWhen, isOverdue, isToday } from "../time";
import Widget from "./Widget";

export type TodoFilter = "open" | "today" | "overdue" | "done";

const TABS: { id: TodoFilter; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "today", label: "Today" },
  { id: "overdue", label: "Overdue" },
  { id: "done", label: "Done" },
];

const PRIORITY = ["", "High", "Normal", "Low"];

function endOfToday(): string {
  const d = new Date();
  d.setHours(17, 0, 0, 0);
  if (d.getTime() < Date.now()) d.setHours(23, 59, 0, 0);
  return d.toISOString();
}

export function applyFilter(todos: Todo[], f: TodoFilter): Todo[] {
  switch (f) {
    case "open":
      return todos.filter((t) => !t.done);
    case "today":
      return todos.filter((t) => !t.done && (isToday(t.due_at) || isOverdue(t.due_at)));
    case "overdue":
      return todos.filter((t) => !t.done && isOverdue(t.due_at));
    case "done":
      return todos
        .filter((t) => t.done)
        .sort((a, b) => (b.done_at ?? "").localeCompare(a.done_at ?? ""));
  }
}

export default function TodosWidget({ filter, setFilter }: { filter: TodoFilter; setFilter: (f: TodoFilter) => void }) {
  const { todos, act } = useData();
  const [title, setTitle] = useState("");
  const shown = applyFilter(todos, filter);

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    // Adding from the Today tab gives the todo a due time today so it stays in view.
    const due = filter === "today" || filter === "overdue" ? endOfToday() : null;
    const saved = await act(() => api.addTodo({ title, due_at: due }));
    if (saved) setTitle("");
  }

  return (
    <Widget
      title="Todos"
      actions={
        <div className="tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={filter === t.id}
              className={filter === t.id ? "tab active" : "tab"}
              onClick={() => setFilter(t.id)}
            >
              {t.label}
              <span className="tab-count">{applyFilter(todos, t.id).length}</span>
            </button>
          ))}
        </div>
      }
    >
      <form onSubmit={add} className="inline-add">
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder={filter === "today" || filter === "overdue" ? "Add a todo for today…" : "Add a todo…"}
          aria-label="New todo"
        />
      </form>
      {shown.length === 0 ? (
        <p className="empty">{filter === "done" ? "Nothing completed yet." : "Nothing here. Nice."}</p>
      ) : (
        <ul className="list dense">
          {shown.map((t) => (
            <li key={t.id} className={t.done ? "done" : ""}>
              <input
                type="checkbox"
                checked={t.done}
                onChange={() => act(() => api.setTodoDone(t.id, !t.done))}
                aria-label={t.done ? `Reopen ${t.title}` : `Complete ${t.title}`}
              />
              <button
                className={`prio-btn p${t.priority}`}
                title={`${PRIORITY[t.priority]} priority. Click to change.`}
                aria-label={`${PRIORITY[t.priority]} priority, change`}
                onClick={() => act(() => api.setTodoPriority(t.id, (t.priority % 3) + 1))}
              />
              <span className="grow truncate" title={t.title}>{t.title}</span>
              {t.due_at && !t.done && (
                <span className={isOverdue(t.due_at) ? "tag critical" : "tag"}>
                  {isOverdue(t.due_at) && "! "}
                  {formatWhen(t.due_at)}
                </span>
              )}
              <button className="ghost row-action" onClick={() => act(() => api.deleteTodo(t.id))} aria-label={`Delete ${t.title}`}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </Widget>
  );
}
