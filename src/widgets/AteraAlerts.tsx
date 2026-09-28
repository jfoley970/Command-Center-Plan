import { useCallback, useEffect, useState, type FormEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorText } from "../api";
import { ago, atera, type AteraAlert, type AteraSnapshot, type Severity } from "../atera";
import { useData } from "../data";
import Widget from "./Widget";
import "./atera.css";

type Filter = "all" | Severity;

const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "critical", label: "Critical" },
  { id: "warning", label: "Warning" },
  { id: "information", label: "Info" },
];

function isSnoozed(a: AteraAlert): boolean {
  return !!a.snoozed_until && new Date(a.snoozed_until).getTime() > Date.now();
}

/** Open alerts from Atera, read-only. Hide a customer here to keep it off the dashboard. */
export default function AteraAlerts() {
  const { act } = useData();
  const [snap, setSnap] = useState<AteraSnapshot | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [showSnoozed, setShowSnoozed] = useState(false);
  const [setup, setSetup] = useState(false);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<number | null>(null);
  const [added, setAdded] = useState<Set<number>>(new Set());
  const [localError, setLocalError] = useState<string | null>(null);

  const load = useCallback(() => {
    atera.alerts().then(setSnap).catch((e) => setLocalError(errorText(e)));
  }, []);

  useEffect(() => {
    load();
    const un = listen("atera-changed", load);
    // Keeps the "5m ago" labels current between polls.
    const t = setInterval(load, 60_000);
    return () => {
      clearInterval(t);
      un.then((f) => f());
    };
  }, [load]);

  async function run(fn: () => Promise<unknown>) {
    setBusy(true);
    setLocalError(null);
    try {
      await fn();
    } catch (e) {
      setLocalError(errorText(e));
    } finally {
      setBusy(false);
      load();
    }
  }

  async function saveKey(e: FormEvent) {
    e.preventDefault();
    await run(() => atera.setKey(key));
    setKey("");
    setSetup(false);
  }

  async function toTodo(a: AteraAlert) {
    const where = [a.customer, a.device].filter(Boolean).join(" / ");
    const saved = await act(() =>
      api.addTodo({
        title: `Atera: ${a.title}${where ? ` (${where})` : ""}`,
        notes: [a.message, a.ticket_id ? `Atera ticket #${a.ticket_id}` : "", `Atera alert #${a.id}`].filter(Boolean).join("\n"),
        priority: a.severity === "critical" ? 1 : 2,
      }),
    );
    if (saved) setAdded((s) => new Set(s).add(a.id));
  }

  const alerts = snap?.alerts ?? [];
  const active = alerts.filter((a) => !isSnoozed(a));
  const snoozedCount = alerts.length - active.length;
  const pool = showSnoozed ? alerts : active;
  const shown = filter === "all" ? pool : pool.filter((a) => a.severity === filter);
  const count = (f: Filter) => (f === "all" ? pool.length : pool.filter((a) => a.severity === f).length);
  const error = localError ?? snap?.error ?? null;
  const needsKey = snap !== null && !snap.has_key;

  const actions = (
    <>
      {!needsKey && !setup && (
        <div className="tabs" role="tablist" aria-label="Filter by severity">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              role="tab"
              aria-selected={filter === f.id}
              className={`tab ${filter === f.id ? "active" : ""}`}
              onClick={() => setFilter(f.id)}
            >
              {f.id !== "all" && <span className={`sev-dot ${f.id}`} aria-hidden="true" />}
              {f.label}
              <span className="tab-count">{count(f.id)}</span>
            </button>
          ))}
        </div>
      )}
      {!needsKey && (
        <button className="ghost" disabled={busy} onClick={() => run(atera.refresh)} title="Check Atera now" aria-label="Refresh alerts">
          {busy ? "…" : "↻"}
        </button>
      )}
      <button className="ghost" onClick={() => setSetup((s) => !s)} title="Atera settings" aria-label="Atera settings">
        ⚙
      </button>
    </>
  );

  return (
    <Widget title="Atera alerts" actions={actions}>
      {error && (
        <div className="atera-error" role="alert">
          {error}
        </div>
      )}

      {(needsKey || setup) && (
        <div className="atera-setup">
          <form className="form-row" onSubmit={saveKey}>
            <input
              className="grow"
              type="password"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              placeholder={snap?.has_key ? "Paste a new Atera API key to replace it" : "Atera API key"}
              aria-label="Atera API key"
              autoComplete="off"
            />
            <button className="primary" type="submit" disabled={!key.trim() || busy}>
              {snap?.has_key ? "Replace" : "Connect"}
            </button>
            {snap?.has_key && (
              <button type="button" className="ghost" onClick={() => run(() => atera.setKey(""))}>
                Disconnect
              </button>
            )}
          </form>
          <p className="muted small">
            In Atera go to Admin, Data management, API and create a token with read access to alerts. It is stored in the
            system keychain. Alerts refresh every 2 minutes and new critical ones pop a notification.
          </p>
          {snap && snap.hidden_customers.length > 0 && (
            <div className="stack-tight">
              <span className="muted small">Hidden customers</span>
              <div className="chips">
                {snap.hidden_customers.map((c) => (
                  <button key={c.id} onClick={() => run(() => atera.setCustomerHidden(c, false))} title="Show this customer again">
                    {c.name || `Customer ${c.id}`} ✕
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {!needsKey && !setup && (
        <>
          {shown.length === 0 ? (
            <p className="empty">
              {snap?.fetched_at ? (filter === "all" ? "No open alerts. All quiet." : "Nothing at this severity.") : "Checking Atera…"}
            </p>
          ) : (
            <ul className="list dense atera-list">
              {shown.map((a) => {
                const expanded = open === a.id;
                return (
                  <li key={a.id} className={`atera-row ${expanded ? "expanded" : ""} ${isSnoozed(a) ? "snoozed" : ""}`}>
                    <div className="atera-line" onClick={() => setOpen(expanded ? null : a.id)}>
                      <span className={`sev-dot ${a.severity}`} title={a.severity} aria-label={a.severity} />
                      <span className="grow truncate" title={a.title}>
                        {a.title || "Untitled alert"}
                        <span className="sub">
                          {[a.customer, a.device, ago(a.created)].filter(Boolean).join(" · ")}
                        </span>
                      </span>
                      {a.ticket_id && <span className="tag">Ticket #{a.ticket_id}</span>}
                      {isSnoozed(a) && <span className="tag">Snoozed</span>}
                      <button
                        className="ghost row-action"
                        disabled={added.has(a.id)}
                        onClick={(e) => {
                          e.stopPropagation();
                          toTodo(a);
                        }}
                        title="Add a todo for this alert"
                      >
                        {added.has(a.id) ? "Added" : "+ Todo"}
                      </button>
                    </div>
                    {expanded && (
                      <div className="atera-detail">
                        {a.message && <p className="atera-message">{a.message}</p>}
                        <div className="row">
                          {a.category && <span className="tag">{a.category}</span>}
                          <span className="grow" />
                          {a.customer_id !== null && (
                            <button
                              className="ghost"
                              onClick={() => run(() => atera.setCustomerHidden({ id: a.customer_id!, name: a.customer }, true))}
                              title="Stop showing alerts for this customer"
                            >
                              Hide {a.customer || "customer"}
                            </button>
                          )}
                        </div>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
          <div className="atera-foot muted small">
            {snap?.fetched_at && <span>Updated {ago(snap.fetched_at)}</span>}
            {snoozedCount > 0 && (
              <button className="link" onClick={() => setShowSnoozed((s) => !s)}>
                {showSnoozed ? "Hide" : "Show"} {snoozedCount} snoozed
              </button>
            )}
            {snap && snap.hidden_count > 0 && (
              <button className="link" onClick={() => setSetup(true)}>
                {snap.hidden_count} from hidden customers
              </button>
            )}
          </div>
        </>
      )}
    </Widget>
  );
}
