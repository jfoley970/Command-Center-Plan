import { useState, type CSSProperties, type FormEvent, type ReactElement } from "react";
import { api, type Project, type Todo } from "../api";
import { useData } from "../data";
import { buildProjectTree, type ProjectNode } from "../projectTree";
import { formatWhen, isOverdue, isToday } from "../time";
import TodoDetail from "./TodoDetail";
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
  const { todos, projects, flagLinks, act } = useData();
  const [title, setTitle] = useState("");
  const [target, setTarget] = useState<Project | null>(null);
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [selected, setSelected] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const flagged = new Set(flagLinks.map((f) => f.todo_id));
  const fromEmail = (t: Todo) => t.email_id != null || flagged.has(t.id);

  function setOpen(id: number, open: boolean) {
    const next = new Set(expanded);
    if (open) next.add(id);
    else next.delete(id);
    setExpanded(next);
  }
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

  function row(t: Todo, depth: number, color?: string) {
    // Inside a project, the dot and checkbox take the project's color and priority shows as how solid the dot is.
    const style = color ? ({ ...indent(depth), "--proj": color } as CSSProperties) : indent(depth);
    const open = expanded.has(t.id);
    const email = fromEmail(t);
    const cls = [t.done && "done", color && "in-project", selected === t.id && "selected"].filter(Boolean).join(" ");
    const item = (
      <li
        key={t.id}
        className={cls}
        style={style}
        tabIndex={0}
        aria-selected={selected === t.id}
        aria-expanded={open}
        onClick={(e) => {
          // Clicks on the row's own controls don't change the selection.
          if ((e.target as HTMLElement).closest("button, input, textarea, a")) return;
          setSelected(t.id);
        }}
        onFocus={(e) => e.target === e.currentTarget && setSelected(t.id)}
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setOpen(t.id, !open);
          } else if (e.key === "ArrowRight") setOpen(t.id, true);
          else if (e.key === "ArrowLeft") setOpen(t.id, false);
          else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            const rows = [...(e.currentTarget.closest("ul")?.querySelectorAll<HTMLElement>("li[tabindex]") ?? [])];
            rows[rows.indexOf(e.currentTarget) + (e.key === "ArrowDown" ? 1 : -1)]?.focus();
          }
        }}
      >
        <button
          className="expand-btn"
          aria-label={open ? `Hide details for ${t.title}` : `Show details for ${t.title}`}
          aria-expanded={open}
          tabIndex={-1}
          onClick={() => {
            setSelected(t.id);
            setOpen(t.id, !open);
          }}
        >
          <svg className="chevron" viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
            <path d="M4 2.5 7.5 6 4 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
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
        {email && (
          <span className="mail-mark" title="From an email" aria-label="From an email">
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <rect x="2" y="3.5" width="12" height="9" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
              <path d="m2.5 4.5 5.5 4 5.5-4" fill="none" stroke="currentColor" strokeWidth="1.3" />
            </svg>
          </span>
        )}
        <span className="grow truncate" title={t.title}>{t.title}</span>
        {!open && t.notes && !email && <span className="notes-mark" title={t.notes}>notes</span>}
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
    if (!open) return item;
    return [
      item,
      <li key={`d-${t.id}`} className="detail-row" style={{ ...style, paddingLeft: `${1.6 + depth * 1.1}rem` }}>
        <TodoDetail todo={t} fromEmail={email} />
      </li>,
    ];
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

  function group(g: Group): (ReactElement | ReactElement[])[] {
    const key = `p:${g.project.id}`;
    const rows: (ReactElement | ReactElement[])[] = [header(key, g.project.name, g.total, g.depth, g.project.color, g.project)];
    if (!collapsed.has(key)) {
      rows.push(...g.children.flatMap(group), ...g.todos.map((t) => row(t, g.depth + 1, g.project.color)));
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
