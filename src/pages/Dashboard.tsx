import { useEffect, useState } from "react";
import ReactGridLayout, { useContainerWidth, verticalCompactor, type Layout } from "react-grid-layout";
import "react-grid-layout/css/styles.css";
import "react-resizable/css/styles.css";
import { api } from "../api";
import { useData } from "../data";
import type { Page } from "../App";
import CommandBar from "../widgets/CommandBar";
import Kpis from "../widgets/Kpis";
import Timeline from "../widgets/Timeline";
import TodosWidget, { type TodoFilter } from "../widgets/TodosWidget";
import RemindersWidget from "../widgets/RemindersWidget";
import AgentConsole from "../widgets/AgentConsole";
import Trend from "../widgets/Trend";
import Integrations from "../widgets/Integrations";

const LAYOUT_KEY = "cc.dashboard.layout.v2";

const DEFAULT_LAYOUT: Layout = [
  { i: "kpis", x: 0, y: 0, w: 12, h: 2, static: true },
  { i: "timeline", x: 0, y: 2, w: 12, h: 4, minH: 3, minW: 6 },
  { i: "todos", x: 0, y: 6, w: 4, h: 9, minH: 4, minW: 3 },
  { i: "reminders", x: 4, y: 6, w: 4, h: 9, minH: 4, minW: 3 },
  { i: "agent", x: 8, y: 6, w: 4, h: 9, minH: 5, minW: 3 },
  { i: "trend", x: 0, y: 15, w: 4, h: 6, minH: 5, minW: 3 },
  { i: "connections", x: 4, y: 15, w: 8, h: 6, minH: 4, minW: 3 },
];

// Layout is a per-device convenience, so browser storage is fine; it must never break the page.
function loadLayout(): Layout {
  try {
    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null") as Layout | null;
    if (!Array.isArray(saved)) return DEFAULT_LAYOUT;
    // Keep constraints from the defaults and add any widget that is new since the save.
    return DEFAULT_LAYOUT.map((d) => {
      const s = saved.find((x) => x.i === d.i);
      return s ? { ...d, x: s.x, y: s.y, w: s.w, h: s.h } : d;
    });
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function saveLayout(layout: Layout) {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout.map(({ i, x, y, w, h }) => ({ i, x, y, w, h }))));
  } catch {
    /* storage unavailable; layout just won't persist */
  }
}

function greeting(d: Date): string {
  const h = d.getHours();
  return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
}

export default function Dashboard({ go }: { go: (p: Page) => void }) {
  const { agents, hasKey, act } = useData();
  const { width, containerRef, mounted } = useContainerWidth();
  const [layout, setLayout] = useState<Layout>(loadLayout);
  const [filter, setFilter] = useState<TodoFilter>("today");
  const [agentId, setAgentId] = useState<number | null>(null);
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(t);
  }, []);

  const planner = agents.find((a) => a.name === "Daily Planner") ?? agents[0];

  return (
    <div className="dashboard">
      <header className="dash-head">
        <div>
          <h1>{greeting(now)}, James</h1>
          <p className="muted">
            {now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })} ·{" "}
            {now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
          </p>
        </div>
        <div className="row">
          <button
            className="ghost"
            onClick={() => {
              setLayout(DEFAULT_LAYOUT);
              saveLayout(DEFAULT_LAYOUT);
            }}
            title="Put every widget back where it started"
          >
            Reset layout
          </button>
          <button
            className="primary"
            disabled={!planner || !hasKey}
            title={hasKey ? "Have Claude prioritize your day" : "Add your API key in Settings first"}
            onClick={() => {
              if (!planner) return;
              setAgentId(planner.id);
              act(() => api.runAgent(planner.id, ""));
            }}
          >
            Plan my day
          </button>
        </div>
      </header>

      {!hasKey && (
        <div className="notice">
          <span className="grow">Agents need your Claude API key.</span>
          <button onClick={() => go("settings")}>Open Settings</button>
        </div>
      )}

      <CommandBar onAsk={setAgentId} />

      <div ref={containerRef} className="grid-wrap">
        {mounted && (
          <ReactGridLayout
            layout={layout}
            width={width}
            gridConfig={{ cols: 12, rowHeight: 36, margin: [14, 14], containerPadding: [0, 0] }}
            dragConfig={{ handle: ".drag-handle", cancel: "button, input, select, textarea" }}
            compactor={verticalCompactor}
            onLayoutChange={(l) => {
              setLayout(l);
              saveLayout(l);
            }}
          >
            <div key="kpis" className="bare"><Kpis filter={filter} setFilter={setFilter} /></div>
            <div key="timeline"><Timeline /></div>
            <div key="todos"><TodosWidget filter={filter} setFilter={setFilter} /></div>
            <div key="reminders"><RemindersWidget /></div>
            <div key="agent">
              <AgentConsole agentId={agentId} setAgentId={setAgentId} onOpenAgents={() => go("agents")} />
            </div>
            <div key="trend"><Trend /></div>
            <div key="connections"><Integrations /></div>
          </ReactGridLayout>
        )}
      </div>
    </div>
  );
}
