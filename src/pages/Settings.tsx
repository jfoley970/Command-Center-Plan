import { useState } from "react";
import type { Page } from "../App";
import { isDesktop, keyStore } from "../transport";
import ServerSettings from "./ServerSettings";
import { allConnectors, type Connector } from "../connectors";
import type { ConnectorState } from "../connectors/registry";

const GROUPS: Connector["group"][] = ["AI", "Email", "Monitoring", "Network", "Infrastructure", "Devices"];

const DOT: Record<ConnectorState, string> = { connected: "ok", attention: "error", off: "idle", planned: "idle" };
const TAG: Record<ConnectorState, string> = { connected: "Connected", attention: "Needs attention", off: "Not set up", planned: "Planned" };

export default function Settings({ go }: { go: (p: Page) => void }) {
  const [open, setOpen] = useState<string | null>(null);
  const connectors = allConnectors();

  return (
    <div className="page">
      <header className="page-head">
        <h1>Settings</h1>
      </header>

      <section className="card stack">
        <div>
          <h2>Connections</h2>
          <p className="muted small">Every API, agent and service the app uses. Keys are stored in {keyStore}.</p>
        </div>
        {GROUPS.map((g) => {
          const items = connectors.filter((c) => c.group === g);
          if (items.length === 0) return null;
          return (
            <div key={g} className="conn-group">
              <h3 className="conn-group-title">{g}</h3>
              <ul className="conn-list">
                {items.map((c) => (
                  <ConnectorRow key={c.id} c={c} open={open === c.id} onToggle={() => setOpen(open === c.id ? null : c.id)} go={go} />
                ))}
              </ul>
            </div>
          );
        })}
      </section>

      {isDesktop && <ServerSettings />}

      <section className="card stack">
        <h2>Shortcuts</h2>
        <ul className="list plain">
          <li><kbd>Ctrl/Cmd</kbd> + <kbd>K</kbd> opens quick actions inside the app.</li>
          {isDesktop ? (
            <>
              <li><kbd>Ctrl/Cmd</kbd> + <kbd>Shift</kbd> + <kbd>Space</kbd> brings the app forward from anywhere.</li>
              <li>Closing the window keeps the app in the tray so reminders still fire. Quit from the tray icon.</li>
            </>
          ) : (
            <li>Reminders and alerts show as browser notifications while this tab is open. The desktop app adds a tray icon and a global shortcut.</li>
          )}
        </ul>
      </section>
    </div>
  );
}

function ConnectorRow({ c, open, onToggle, go }: { c: Connector; open: boolean; onToggle: () => void; go: (p: Page) => void }) {
  const status = c.useStatus();
  const Panel = c.Panel;
  return (
    <li className={`conn-row ${open ? "open" : ""}`}>
      <button className="conn-head" onClick={Panel ? onToggle : undefined} disabled={!Panel} aria-expanded={Panel ? open : undefined}>
        <span className={`status-dot ${DOT[status.state]}`} aria-hidden="true" />
        <span className="grow conn-text">
          <span className="conn-name">{c.name}</span>
          <span className="sub truncate" title={status.detail}>{status.state === "planned" ? c.description : status.detail}</span>
        </span>
        <span className={`tag ${status.state === "attention" ? "bad" : ""}`}>{TAG[status.state]}</span>
        {Panel && <span className="conn-chevron" aria-hidden="true">{open ? "▾" : "▸"}</span>}
      </button>
      {open && Panel && (
        <div className="conn-panel">
          <p className="muted small">{c.description}</p>
          <Panel go={go} />
        </div>
      )}
    </li>
  );
}
