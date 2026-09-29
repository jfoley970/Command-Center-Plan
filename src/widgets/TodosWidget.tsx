import { useState, type FormEvent, type ReactElement } from "react";
import { api, type Project, type Todo } from "../api";
import { useData } from "../data";
import { buildProjectTree, type ProjectNode } from "../projectTree";
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

const COLLAPSED_KEY = "cc.todos.collapsed";
const NO_PROJECT = "none";

function loadCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function saveCollapsed(keys: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...keys]));
  } catch {
    // Collapse state is a convenience; losing it is fine.
  }
}

/** A project in the widget's tree, holding only the todos the current tab shows. */
type Group = { project: Project; depth: number; todos: Todo[]; children: Group[]; total: number };

function toGroups(nodes: ProjectNode[], byProject: Map<number, Todo[]>): Group[] {
  return nodes
    .map((n) => {
      const children = toGroups(n.children, byProject);
      const todos = byProject.get(n.project.id) ?? [];
      return { project: n.project, depth: n.depth, todos, children, total: todos.length + children.reduce((a, c) => a + c.total, 0) };
    })
    .filter((g) => g.total > 0);
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
  const { todos, projects, act } = useData();
  const [title, setTitle] = useState("");
  const [target, setTarget] = useState<Project | null>(null);
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const shown = applyFilter(todos, filter);

  const known = new Set(projects.map((p) => p.id));
  const byProject = new Map<number, Todo[]>();
  const loose: Todo[] = [];
  for (const t of shown) {
    if (t.project_id != null && known.has(t.project_id)) byProject.set(t.project_id, [...(byProject.get(t.project_id) ?? []), t]);
    else loose.push(t);
  }
  const groups = toGroups(buildProjectTree(projects), byProject);

  function toggle(key: string) {
    const next = new Set(collapsed);
    if (!next.delete(key)) next.add(key);
    setCollapsed(next);
    saveCollapsed(next);
  }

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    // Adding from the Today tab gives the todo a due time today so it stays in view.
    const due = filter === "today" || filter === "overdue" ? endOfToday() : null;
    const saved = await act(() => api.addTodo({ title, due_at: due, project_id: target?.id ?? null }));
    if (saved) {
      setTitle("");
      // Make sure the new todo is visible in its group.
      if (target && collapsed.has(`p:${target.id}`)) toggle(`p:${target.id}`);
    }
  }

  function addTo(p: Project) {
    setTarget(p);
    document.getElementById("todo-add")?.focus();
  }

  function row(t: Todo, depth: number) {
    return (
      <li key={t.id} className={t.done ? "done" : ""} style={indent(depth)}>
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
    );
  }

  function header(key: string, label: string, count: number, depth: number, color?: string, project?: Project) {
    const open = !collapsed.has(key);
    return (
      <li key={`h-${key}`} className="tree-head" style={indent(depth)}>
        <button className="tree-toggle" aria-expanded={open} onClick={() => toggle(key)}>
          <svg className="chevron" viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
            <path d="M4 2.5 7.5 6 4 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {color ? <span className="dot" style={{ background: color }} /> : <span className="dot none" />}
          <span className="grow truncate">{label}</span>
          <span className="tab-count">{count}</span>
        </button>
        {project && filter !== "done" && (
          <button className="ghost row-action" onClick={() => addTo(project)} aria-label={`Add a todo to ${project.name}`} title={`Add to ${project.name}`}>
            +
          </button>
        )}
      </li>
    );
  }

  function group(g: Group): ReactElement[] {
    const key = `p:${g.project.id}`;
    const rows = [header(key, g.project.name, g.total, g.depth, g.project.color, g.project)];
    if (!collapsed.has(key)) {
      rows.push(...g.children.flatMap(group), ...g.todos.map((t) => row(t, g.depth + 1)));
    }
    return rows;
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
        {target && (
          <button type="button" className="target-chip" onClick={() => setTarget(null)} title="Add without a project" aria-label={`Adding to ${target.name}. Clear`}>
            <span className="dot" style={{ background: target.color }} />
            {target.name} ✕
          </button>
        )}
        <input
          id="todo-add"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setTarget(null)}
          placeholder={
            target ? `Add a todo to ${target.name}…` : filter === "today" || filter === "overdue" ? "Add a todo for today…" : "Add a todo…"
          }
          aria-label="New todo"
        />
      </form>
      {shown.length === 0 ? (
        <p className="empty">{filter === "done" ? "Nothing completed yet." : "Nothing here. Nice."}</p>
      ) : groups.length === 0 ? (
        // Nothing on this tab belongs to a project, so a lone "No project" group would only add a click.
        <ul className="list dense">{loose.map((t) => row(t, 0))}</ul>
      ) : (
        <ul className="list dense tree">
          {groups.flatMap(group)}
          {loose.length > 0 && [
            header(NO_PROJECT, "No project", loose.length, 0),
            ...(collapsed.has(NO_PROJECT) ? [] : loose.map((t) => row(t, 1))),
          ]}
        </ul>
      )}
    </Widget>
  );
}

function indent(depth: number) {
  return depth ? { paddingLeft: `${0.25 + depth * 1.1}rem` } : undefined;
}
