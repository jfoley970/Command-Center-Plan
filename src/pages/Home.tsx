import { useState } from "react";
import { api } from "../api";
import { useData } from "../data";
import { formatWhen, isOverdue, isToday } from "../time";
import type { Page } from "../App";
import { RunCard } from "./Agents";

function greeting(): string {
  const h = new Date().getHours();
  return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
}

export default function Home({ go }: { go: (p: Page) => void }) {
  const { todos, reminders, agents, runs, hasKey, act } = useData();
  const [planning, setPlanning] = useState(false);

  const open = todos.filter((t) => !t.done);
  const overdue = open.filter((t) => isOverdue(t.due_at));
  const dueToday = open.filter((t) => isToday(t.due_at));
  const upcoming = reminders.filter((r) => !r.fired).slice(0, 5);
  const remindersToday = reminders.filter((r) => !r.fired && isToday(r.remind_at));
  const runsToday = runs.filter((r) => isToday(r.started_at));
  const topTodos = open.slice(0, 6);
  const planner = agents.find((a) => a.name === "Daily Planner") ?? agents[0];
  const latestPlan = planner ? runs.find((r) => r.agent_id === planner.id) : undefined;

  async function planDay() {
    if (!planner) return;
    setPlanning(true);
    await act(() => api.runAgent(planner.id, ""));
    setPlanning(false);
  }

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>{greeting()}, James</h1>
          <p className="muted">
            {new Date().toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}
          </p>
        </div>
        <div className="row">
          <button className="primary" onClick={planDay} disabled={!planner || !hasKey || planning}>
            {planning ? "Planning…" : "Plan my day"}
          </button>
        </div>
      </header>

      {!hasKey && (
        <div className="notice">
          Add your Claude API key in <a onClick={() => go("settings")}>Settings</a> to run agents.
        </div>
      )}

      <section className="stats">
        <Stat label="Open todos" value={open.length} onClick={() => go("todos")} />
        <Stat label="Overdue" value={overdue.length} tone={overdue.length ? "bad" : undefined} onClick={() => go("todos")} />
        <Stat label="Due today" value={dueToday.length} onClick={() => go("todos")} />
        <Stat label="Reminders today" value={remindersToday.length} onClick={() => go("reminders")} />
        <Stat label="Agent runs today" value={runsToday.length} onClick={() => go("agents")} />
      </section>

      <div className="grid">
        <section className="card">
          <h2>Todos</h2>
          {topTodos.length === 0 ? (
            <p className="muted">Nothing open. Add one with Ctrl/Cmd+K.</p>
          ) : (
            <ul className="list">
              {topTodos.map((t) => (
                <li key={t.id}>
                  <input
                    type="checkbox"
                    checked={t.done}
                    onChange={() => act(() => api.setTodoDone(t.id, !t.done))}
                  />
                  <span className={`prio p${t.priority}`} />
                  <span className="grow">{t.title}</span>
                  {t.due_at && (
                    <span className={isOverdue(t.due_at) ? "tag bad" : "tag"}>{formatWhen(t.due_at)}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="card">
          <h2>Upcoming reminders</h2>
          {upcoming.length === 0 ? (
            <p className="muted">No reminders scheduled.</p>
          ) : (
            <ul className="list">
              {upcoming.map((r) => (
                <li key={r.id}>
                  <span className="grow">{r.title}</span>
                  <span className="tag">{formatWhen(r.remind_at)}</span>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="card span2">
          <h2>Today's plan</h2>
          {latestPlan ? <RunCard run={latestPlan} /> : <p className="muted">Press "Plan my day" to get one.</p>}
        </section>

        <section className="card soon">
          <h2>Email digest</h2>
          <p className="muted">Coming in milestone 2: a summary of what needs a reply, what is FYI, and what to skip.</p>
        </section>

        <section className="card soon">
          <h2>ESXi host</h2>
          <p className="muted">Coming in milestone 2: VM power state, host health and datastore space.</p>
        </section>
      </div>
    </div>
  );
}

function Stat(props: { label: string; value: number; tone?: "bad"; onClick: () => void }) {
  return (
    <button className={`stat ${props.tone ?? ""}`} onClick={props.onClick}>
      <span className="stat-value">{props.value}</span>
      <span className="stat-label">{props.label}</span>
    </button>
  );
}
