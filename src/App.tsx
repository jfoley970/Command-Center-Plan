import { useEffect, useState, type ReactNode } from "react";
import { Command } from "cmdk";
import { listen } from "./transport";
import { api } from "./api";
import { DataProvider, useData } from "./data";
import { minutesFromNow, quick, tomorrowAt9 } from "./quick";
import Dashboard from "./pages/Dashboard";
import Inbox from "./pages/Inbox";
import Projects from "./pages/Projects";
import Todos from "./pages/Todos";
import Reminders from "./pages/Reminders";
import Agents from "./pages/Agents";
import Settings from "./pages/Settings";
import ReminderAlerts from "./ReminderAlerts";
import TopBar from "./TopBar";
import "./styles.css";

export type Page = "dashboard" | "inbox" | "projects" | "todos" | "reminders" | "agents" | "settings";

const icon = (d: ReactNode) => (
  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {d}
  </svg>
);

const NAV: { id: Page; label: string; icon: ReactNode }[] = [
  { id: "dashboard", label: "Dashboard", icon: icon(<><rect x="3" y="3" width="7" height="9" rx="1.5" /><rect x="14" y="3" width="7" height="5" rx="1.5" /><rect x="14" y="12" width="7" height="9" rx="1.5" /><rect x="3" y="16" width="7" height="5" rx="1.5" /></>) },
  { id: "inbox", label: "Inbox", icon: icon(<><path d="M22 12h-6l-2 3h-4l-2-3H2" /><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" /></>) },
  { id: "projects", label: "Projects", icon: icon(<><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></>) },
  { id: "todos", label: "Todos", icon: icon(<><path d="M9 6h11M9 12h11M9 18h11" /><path d="m3.5 6 1.5 1.5L7.5 5M3.5 12l1.5 1.5L7.5 11M3.5 18l1.5 1.5L7.5 17" /></>) },
  { id: "reminders", label: "Reminders", icon: icon(<><path d="M6 8a6 6 0 1 1 12 0c0 7 3 9 3 9H3s3-2 3-9" /><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" /></>) },
  { id: "agents", label: "Agents", icon: icon(<><rect x="4" y="7" width="16" height="12" rx="3" /><path d="M12 3v4M9 12h.01M15 12h.01M9 16h6" /></>) },
  { id: "settings", label: "Settings", icon: icon(<><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></>) },
];

export default function App() {
  return (
    <DataProvider>
      <Shell />
    </DataProvider>
  );
}

function Shell() {
  const { error, setError } = useData();
  const [page, setPage] = useState<Page>("dashboard");
  const [agentId, setAgentId] = useState<number | null>(null);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [inboxId, setInboxId] = useState<number | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    const unlisten = listen("open-palette", () => setPaletteOpen(true));
    return () => {
      window.removeEventListener("keydown", onKey);
      unlisten.then((f) => f());
    };
  }, []);

  return (
    <div className="app">
      <nav className="rail" aria-label="Main">
        <div className="rail-logo" aria-hidden="true">CC</div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={page === n.id ? "rail-btn active" : "rail-btn"}
            onClick={() => setPage(n.id)}
            title={n.label}
            aria-label={n.label}
            aria-current={page === n.id ? "page" : undefined}
          >
            {n.icon}
            <span>{n.label}</span>
          </button>
        ))}
        <button className="rail-btn rail-bottom" onClick={() => setPaletteOpen(true)} title="Quick actions (Ctrl/Cmd+K)" aria-label="Quick actions">
          {icon(<><path d="M18 3a3 3 0 0 0-3 3v12a3 3 0 0 0 3 3 3 3 0 0 0 3-3 3 3 0 0 0-3-3H6a3 3 0 0 0-3 3 3 3 0 0 0 3 3 3 3 0 0 0 3-3V6a3 3 0 0 0-3-3 3 3 0 0 0-3 3 3 3 0 0 0 3 3h12a3 3 0 0 0 3-3 3 3 0 0 0-3-3z" /></>)}
          <span>⌘K</span>
        </button>
      </nav>

      <main className="main">
        {error && (
          <div className="notice bad" role="alert">
            <span className="grow">{error}</span>
            <button className="ghost" onClick={() => setError(null)} aria-label="Dismiss">✕</button>
          </div>
        )}
        {page === "dashboard" && <Dashboard go={setPage} />}
        {page === "inbox" && <Inbox selectedId={inboxId} onSelect={setInboxId} go={setPage} />}
        {page === "projects" && <Projects selectedId={projectId} onSelect={setProjectId} />}
        {page === "todos" && <Todos />}
        {page === "reminders" && <Reminders />}
        {page === "agents" && <Agents selectedId={agentId} onSelect={setAgentId} />}
        {page === "settings" && <Settings go={setPage} />}
      </main>

      <ReminderAlerts />
      <TopBar />

      <Palette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        go={setPage}
        showAgent={(id) => {
          setAgentId(id);
          setPage("agents");
        }}
      />
    </div>
  );
}

function Palette(props: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  go: (p: Page) => void;
  showAgent: (id: number) => void;
}) {
  const { agents, hasKey, act } = useData();
  const [search, setSearch] = useState("");
  const text = search.trim();

  function done(fn?: () => void) {
    fn?.();
    setSearch("");
    props.onOpenChange(false);
  }

  return (
    <Command.Dialog open={props.open} onOpenChange={props.onOpenChange} label="Quick actions" className="palette">
      <Command.Input value={search} onValueChange={setSearch} placeholder="Type a command, or text to add as a todo…" />
      <Command.List>
        {!text && <Command.Empty>No matching action.</Command.Empty>}

        {text && (
          <Command.Group heading="Add" forceMount>
            <Command.Item forceMount value={`add-todo ${text}`} onSelect={() => done(() => act(() => quick.todo(text)))}>
              Add todo: <strong>{text}</strong>
            </Command.Item>
            <Command.Item forceMount value={`remind-30 ${text}`} onSelect={() => done(() => act(() => quick.remind(text, minutesFromNow(30))))}>
              Remind me in 30 minutes: <strong>{text}</strong>
            </Command.Item>
            <Command.Item forceMount value={`remind-tomorrow ${text}`} onSelect={() => done(() => act(() => quick.remind(text, tomorrowAt9())))}>
              Remind me tomorrow at 9 AM: <strong>{text}</strong>
            </Command.Item>
          </Command.Group>
        )}

        <Command.Group heading="Agents" forceMount={!!text}>
          {agents.map((a) => (
            <Command.Item
              key={a.id}
              forceMount={!!text}
              value={`run ${a.name} ${text}`}
              disabled={!hasKey}
              onSelect={() =>
                done(() => {
                  props.showAgent(a.id);
                  act(() => api.runAgent(a.id, text));
                })
              }
            >
              Run {a.name}
              {text && <span className="muted"> with “{text}”</span>}
            </Command.Item>
          ))}
        </Command.Group>

        <Command.Group heading="Go to">
          {NAV.map((n) => (
            <Command.Item key={n.id} value={`go ${n.label}`} onSelect={() => done(() => props.go(n.id))}>
              {n.label}
            </Command.Item>
          ))}
        </Command.Group>
      </Command.List>
    </Command.Dialog>
  );
}
