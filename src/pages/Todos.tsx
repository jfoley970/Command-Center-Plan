import { useState, type FormEvent } from "react";
import { openUrl } from "../transport";
import { api } from "../api";
import { useData } from "../data";
import { formatWhen, fromLocalInput, isOverdue } from "../time";

export default function Todos() {
  const { todos, projects, flagLinks, act } = useData();
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState(2);
  const [due, setDue] = useState("");
  const [projectId, setProjectId] = useState("");
  const [showDone, setShowDone] = useState(false);

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    const saved = await act(() =>
      api.addTodo({ title, priority, due_at: due ? fromLocalInput(due) : null, project_id: projectId ? Number(projectId) : null }),
    );
    if (saved) {
      setTitle("");
      setDue("");
      setPriority(2);
    }
  }

  const projectOf = (id: number | null) => projects.find((p) => p.id === id);
  const emailOf = (id: number) => flagLinks.find((f) => f.todo_id === id && f.web_link)?.web_link;
  const activeProjects = projects.filter((p) => !p.archived);

  const open = todos.filter((t) => !t.done);
  const done = todos.filter((t) => t.done);

  return (
    <div className="page">
      <header className="page-head">
        <h1>Todos</h1>
        <label className="row muted">
          <input type="checkbox" checked={showDone} onChange={(e) => setShowDone(e.target.checked)} />
          Show completed ({done.length})
        </label>
      </header>

      <form className="card form-row" onSubmit={add}>
        <input
          className="grow"
          placeholder="What needs doing?"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          autoFocus
        />
        <select value={priority} onChange={(e) => setPriority(Number(e.target.value))} aria-label="Priority">
          <option value={1}>High</option>
          <option value={2}>Normal</option>
          <option value={3}>Low</option>
        </select>
        <input type="datetime-local" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due" />
        {activeProjects.length > 0 && (
          <select value={projectId} onChange={(e) => setProjectId(e.target.value)} aria-label="Project">
            <option value="">No project</option>
            {activeProjects.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        )}
        <button className="primary" type="submit">Add</button>
      </form>

      <section className="card">
        {open.length === 0 && <p className="muted">All clear.</p>}
        <ul className="list">
          {[...open, ...(showDone ? done : [])].map((t) => (
            <li key={t.id} className={t.done ? "done" : ""}>
              <input type="checkbox" checked={t.done} onChange={() => act(() => api.setTodoDone(t.id, !t.done))} />
              <span className={`prio p${t.priority}`} title={["", "High", "Normal", "Low"][t.priority]} />
              <span className="grow">{t.title}</span>
              {projectOf(t.project_id) && (
                <span className="tag project-tag">
                  <span className="dot" style={{ background: projectOf(t.project_id)!.color }} />
                  {projectOf(t.project_id)!.name}
                </span>
              )}
              {emailOf(t.id) && (
                <button className="ghost" onClick={() => openUrl(emailOf(t.id)!)} title="Open the flagged email in Outlook" aria-label="Open email">✉</button>
              )}
              {t.due_at && <span className={!t.done && isOverdue(t.due_at) ? "tag bad" : "tag"}>{formatWhen(t.due_at)}</span>}
              <button className="ghost" onClick={() => act(() => api.deleteTodo(t.id))} aria-label="Delete">
                ✕
              </button>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
