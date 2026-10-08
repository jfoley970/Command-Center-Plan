import { useState, type FormEvent } from "react";
import { api, type Project } from "../api";
import { useData } from "../data";
import { buildProjectTree, flattenTree, subtreeIds } from "../projectTree";
import { formatWhen, fromLocalInput, isOverdue, toLocalInput } from "../time";

const COLORS = ["#4c8dff", "#0ca30c", "#d95926", "#fab219", "#b36be0", "#e66767", "#2bb3b3", "#8a909b"];

type Draft = { id?: number; name: string; description: string; color: string; archived: boolean; parent_id: number | null };

function inAnHour(): string {
  const d = new Date(Date.now() + 60 * 60 * 1000);
  d.setSeconds(0, 0);
  return toLocalInput(d);
}

export default function Projects({ selectedId, onSelect }: { selectedId: number | null; onSelect: (id: number | null) => void }) {
  const { projects, todos, act } = useData();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [showArchived, setShowArchived] = useState(false);

  const visible = flattenTree(buildProjectTree(projects.filter((p) => showArchived || !p.archived)));
  const project = projects.find((p) => p.id === selectedId) ?? visible[0]?.project;
  // A project can't move under itself or one of its own sub-projects.
  const draftNode = draft?.id ? flattenTree(buildProjectTree(projects)).find((n) => n.project.id === draft.id) : undefined;
  const blocked = new Set(draftNode ? subtreeIds(draftNode) : []);
  const parentChoices = flattenTree(buildProjectTree(projects.filter((p) => !p.archived || p.id === draft?.parent_id))).filter(
    (n) => !blocked.has(n.project.id),
  );
  const openCount = (id: number) => todos.filter((t) => t.project_id === id && !t.done).length;

  function edit(p?: Project) {
    setDraft(
      p
        ? { id: p.id, name: p.name, description: p.description, color: p.color, archived: p.archived, parent_id: p.parent_id }
        : { name: "", description: "", color: COLORS[projects.length % COLORS.length], archived: false, parent_id: null },
    );
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!draft) return;
    const saved = await act(() => api.saveProject(draft));
    if (saved) {
      setDraft(null);
      onSelect(saved.id);
    }
  }

  async function remove(p: Project) {
    if (!confirm(`Delete the project "${p.name}"? Its tasks and reminders are kept, just unassigned, and any sub-projects move up a level.`)) return;
    await act(() => api.deleteProject(p.id));
    onSelect(null);
  }

  return (
    <div className="page split">
      <aside className="card agent-list">
        <div className="row between">
          <h2>Projects</h2>
          <button onClick={() => edit()}>New</button>
        </div>
        {visible.length === 0 && <p className="muted">No projects yet.</p>}
        <ul className="list dense">
          {visible.map(({ project: p, depth }) => (
            <li
              key={p.id}
              style={depth ? { paddingLeft: `${0.25 + depth * 1.1}rem` } : undefined}
              className={p.id === project?.id ? "active" : ""}
              onClick={() => {
                setDraft(null);
                onSelect(p.id);
              }}
            >
              <span className="dot" style={{ background: p.color }} />
              <span className={p.archived ? "grow muted" : "grow"}>{p.name}</span>
              {openCount(p.id) > 0 && <span className="tag">{openCount(p.id)}</span>}
            </li>
          ))}
        </ul>
        {projects.some((p) => p.archived) && (
          <label className="row muted small">
            <input type="checkbox" checked={showArchived} onChange={(e) => setShowArchived(e.target.checked)} />
            Show archived
          </label>
        )}
      </aside>

      <div className="grow stack">
        {draft ? (
          <form className="card stack" onSubmit={save}>
            <h2>{draft.id ? "Edit project" : "New project"}</h2>
            <label>
              Name
              <input value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} autoFocus />
            </label>
            <label>
              Inside
              <select
                value={draft.parent_id ?? ""}
                onChange={(e) => setDraft({ ...draft, parent_id: e.target.value ? Number(e.target.value) : null })}
              >
                <option value="">Top level</option>
                {parentChoices.map(({ project: p, depth }) => (
                  <option key={p.id} value={p.id}>
                    {"\u00a0\u00a0".repeat(depth)}
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Description
              <textarea rows={3} value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} placeholder="What is this project about?" />
            </label>
            <div className="row" role="radiogroup" aria-label="Color">
              {COLORS.map((c) => (
                <button
                  key={c}
                  type="button"
                  role="radio"
                  aria-checked={draft.color === c}
                  aria-label={c}
                  className={draft.color === c ? "swatch active" : "swatch"}
                  style={{ background: c }}
                  onClick={() => setDraft({ ...draft, color: c })}
                />
              ))}
            </div>
            <div className="row">
              <button className="primary" type="submit">Save</button>
              <button type="button" onClick={() => setDraft(null)}>Cancel</button>
            </div>
          </form>
        ) : project ? (
          <ProjectDetail project={project} onEdit={() => edit(project)} onDelete={() => remove(project)} />
        ) : (
          <section className="card">
            <h2>Start your first project</h2>
            <p className="muted">Projects group related tasks and reminders. Everything still shows up on your dashboard and timeline.</p>
            <button className="primary" onClick={() => edit()}>New project</button>
          </section>
        )}
      </div>
    </div>
  );
}

function ProjectDetail({ project, onEdit, onDelete }: { project: Project; onEdit: () => void; onDelete: () => void }) {
  const { todos, reminders, act } = useData();
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState(2);
  const [due, setDue] = useState("");
  const [remTitle, setRemTitle] = useState("");
  const [remWhen, setRemWhen] = useState(inAnHour);

  const tasks = todos.filter((t) => t.project_id === project.id);
  const open = tasks.filter((t) => !t.done);
  const done = tasks.filter((t) => t.done);
  const upcoming = reminders.filter((r) => r.project_id === project.id && !r.fired);
  const pct = tasks.length ? Math.round((done.length / tasks.length) * 100) : 0;

  async function addTask(e: FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    const saved = await act(() =>
      api.addTodo({ title, priority, due_at: due ? fromLocalInput(due) : null, project_id: project.id }),
    );
    if (saved) {
      setTitle("");
      setDue("");
      setPriority(2);
    }
  }

  async function addReminder(e: FormEvent) {
    e.preventDefault();
    if (!remTitle.trim() || !remWhen) return;
    const saved = await act(() =>
      api.addReminder({ title: remTitle, remind_at: fromLocalInput(remWhen), repeat: "none", project_id: project.id }),
    );
    if (saved) {
      setRemTitle("");
      setRemWhen(inAnHour());
    }
  }

  return (
    <>
      <section className="card stack">
        <div className="row between">
          <div className="row">
            <span className="dot lg" style={{ background: project.color }} />
            <h1>{project.name}</h1>
            {project.archived && <span className="tag">Archived</span>}
          </div>
          <div className="row">
            <button onClick={onEdit}>Edit</button>
            <button onClick={() => act(() => api.saveProject({ ...project, archived: !project.archived }))}>
              {project.archived ? "Unarchive" : "Archive"}
            </button>
            <button className="ghost" onClick={onDelete}>Delete</button>
          </div>
        </div>
        {project.description && <p className="muted">{project.description}</p>}
        <div className="progress" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label="Tasks done">
          <span style={{ width: `${pct}%`, background: project.color }} />
        </div>
        <p className="muted small">
          {done.length} of {tasks.length} tasks done · {upcoming.length} upcoming reminder{upcoming.length === 1 ? "" : "s"}
        </p>
      </section>

      <section className="card stack">
        <h2>Tasks</h2>
        <form className="form-row" onSubmit={addTask}>
          <input className="grow" placeholder="Add a task…" value={title} onChange={(e) => setTitle(e.target.value)} />
          <select value={priority} onChange={(e) => setPriority(Number(e.target.value))} aria-label="Priority">
            <option value={1}>High</option>
            <option value={2}>Normal</option>
            <option value={3}>Low</option>
          </select>
          <input type="datetime-local" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due" />
          <button className="primary" type="submit">Add</button>
        </form>
        {tasks.length === 0 && <p className="muted">No tasks yet.</p>}
        <ul className="list">
          {[...open, ...done].map((t) => (
            <li key={t.id} className={t.done ? "done" : ""}>
              <input type="checkbox" checked={t.done} onChange={() => act(() => api.setTodoDone(t.id, !t.done))} />
              <span className={`prio p${t.priority}`} title={["", "High", "Normal", "Low"][t.priority]} />
              <span className="grow">{t.title}</span>
              {t.due_at && <span className={!t.done && isOverdue(t.due_at) ? "tag bad" : "tag"}>{formatWhen(t.due_at)}</span>}
              <button className="ghost" onClick={() => act(() => api.deleteTodo(t.id))} aria-label="Delete">✕</button>
            </li>
          ))}
        </ul>
      </section>

      <section className="card stack">
        <h2>Reminders</h2>
        <form className="form-row" onSubmit={addReminder}>
          <input className="grow" placeholder="Remind me to…" value={remTitle} onChange={(e) => setRemTitle(e.target.value)} />
          <input type="datetime-local" value={remWhen} onChange={(e) => setRemWhen(e.target.value)} aria-label="When" />
          <button className="primary" type="submit">Add</button>
        </form>
        {upcoming.length === 0 && <p className="muted">Nothing scheduled.</p>}
        <ul className="list">
          {upcoming.map((r) => (
            <li key={r.id}>
              <span className="grow">{r.title}</span>
              {r.repeat !== "none" && <span className="tag">{r.repeat}</span>}
              <span className="tag">{formatWhen(r.remind_at)}</span>
              <button className="ghost" onClick={() => act(() => api.deleteReminder(r.id))} aria-label="Delete">✕</button>
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}
