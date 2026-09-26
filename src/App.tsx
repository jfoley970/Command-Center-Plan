import { useEffect, useState } from "react";
import { Command } from "cmdk";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { DataProvider, useData } from "./data";
import Home from "./pages/Home";
import Todos from "./pages/Todos";
import Reminders from "./pages/Reminders";
import Agents from "./pages/Agents";
import Settings from "./pages/Settings";
import "./styles.css";

export type Page = "home" | "todos" | "reminders" | "agents" | "settings";

const NAV: { id: Page; label: string }[] = [
  { id: "home", label: "Home" },
  { id: "todos", label: "Todos" },
  { id: "reminders", label: "Reminders" },
  { id: "agents", label: "Agents" },
  { id: "settings", label: "Settings" },
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
  const [page, setPage] = useState<Page>("home");
  const [agentId, setAgentId] = useState<number | null>(null);
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
      <nav className="sidebar">
        <div className="brand">Command Center</div>
        {NAV.map((n) => (
          <button key={n.id} className={page === n.id ? "nav active" : "nav"} onClick={() => setPage(n.id)}>
            {n.label}
          </button>
        ))}
        <button className="nav palette-hint" onClick={() => setPaletteOpen(true)}>
          Quick actions <kbd>⌘/Ctrl K</kbd>
        </button>
      </nav>

      <main className="main">
        {error && (
          <div className="notice bad" role="alert">
            <span className="grow">{error}</span>
            <button className="ghost" onClick={() => setError(null)} aria-label="Dismiss">✕</button>
          </div>
        )}
        {page === "home" && <Home go={setPage} />}
        {page === "todos" && <Todos />}
        {page === "reminders" && <Reminders />}
        {page === "agents" && <Agents selectedId={agentId} onSelect={setAgentId} />}
        {page === "settings" && <Settings />}
      </main>

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

function tomorrowAt9(): Date {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  d.setHours(9, 0, 0, 0);
  return d;
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

  const remind = (title: string, at: Date) => act(() => api.addReminder({ title, remind_at: at.toISOString(), repeat: "none" }));

  return (
    <Command.Dialog open={props.open} onOpenChange={props.onOpenChange} label="Quick actions" className="palette">
      <Command.Input value={search} onValueChange={setSearch} placeholder="Type a command, or text to add as a todo…" />
      <Command.List>
        {!text && <Command.Empty>No matching action.</Command.Empty>}

        {text && (
          <Command.Group heading="Add" forceMount>
            <Command.Item forceMount value={`add-todo ${text}`} onSelect={() => done(() => act(() => api.addTodo({ title: text })))}>
              Add todo: <strong>{text}</strong>
            </Command.Item>
            <Command.Item
              forceMount
              value={`remind-30 ${text}`}
              onSelect={() => done(() => remind(text, new Date(Date.now() + 30 * 60 * 1000)))}
            >
              Remind me in 30 minutes: <strong>{text}</strong>
            </Command.Item>
            <Command.Item forceMount value={`remind-tomorrow ${text}`} onSelect={() => done(() => remind(text, tomorrowAt9()))}>
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
