import { useData } from "../data";
import { isOverdue, isToday } from "../time";
import type { TodoFilter } from "./TodosWidget";

/** Headline counts. Each tile filters the todo list or jumps to its widget. */
export default function Kpis({ filter, setFilter }: { filter: TodoFilter; setFilter: (f: TodoFilter) => void }) {
  const { todos, reminders, runs } = useData();
  const open = todos.filter((t) => !t.done);
  const overdue = open.filter((t) => isOverdue(t.due_at)).length;
  const weekAgo = Date.now() - 7 * 86400_000;
  const doneWeek = todos.filter((t) => t.done && t.done_at && new Date(t.done_at).getTime() >= weekAgo).length;
  const remindersLeft = reminders.filter((r) => !r.fired && isToday(r.remind_at)).length;
  const runsToday = runs.filter((r) => isToday(r.started_at)).length;

  const tiles: { key: TodoFilter | null; label: string; value: number; tone?: "critical" }[] = [
    { key: "open", label: "Open todos", value: open.length },
    { key: "overdue", label: "Overdue", value: overdue, tone: overdue ? "critical" : undefined },
    { key: "today", label: "Due today", value: open.filter((t) => isToday(t.due_at)).length },
    { key: "done", label: "Done this week", value: doneWeek },
    { key: null, label: "Reminders left today", value: remindersLeft },
    { key: null, label: "Agent runs today", value: runsToday },
  ];

  return (
    <div className="kpis">
      {tiles.map((t) => (
        <button
          key={t.label}
          className={`kpi ${t.tone ?? ""} ${t.key && t.key === filter ? "selected" : ""}`}
          onClick={() => t.key && setFilter(t.key)}
          disabled={!t.key}
          title={t.key ? `Show ${t.label.toLowerCase()} in the todo list` : undefined}
        >
          <span className="kpi-value">
            {t.tone === "critical" && <span className="status-icon" aria-hidden="true">!</span>}
            {t.value}
          </span>
          <span className="kpi-label">{t.label}</span>
        </button>
      ))}
    </div>
  );
}
