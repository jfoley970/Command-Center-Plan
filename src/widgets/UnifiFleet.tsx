import { useState, type FormEvent } from "react";
import { api, errorText } from "../api";
import { ago } from "../atera";
import { useData } from "../data";
import { useLiveSnapshot } from "../live";
import { keyStore } from "../transport";
import { unifi, type UnifiSite } from "../unifi";
import Widget from "./Widget";
import "./atera.css";
import "./unifi.css";

const LABEL = { ok: "Healthy", warning: "Needs attention", down: "Down" } as const;

/** Health for every UniFi site on the account, read-only. Hide a site here to keep it off the dashboard. */
export default function UnifiFleet() {
  const { act } = useData();
  const { snap, error: loadError, reload } = useLiveSnapshot(unifi.fleet, "unifi-changed");
  const [setup, setSetup] = useState(false);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [added, setAdded] = useState<Set<string>>(new Set());
  const [localError, setLocalError] = useState<string | null>(null);

  async function run(fn: () => Promise<unknown>) {
    setBusy(true);
    setLocalError(null);
    try {
      await fn();
    } catch (e) {
      setLocalError(errorText(e));
    } finally {
      setBusy(false);
      reload();
    }
  }

  async function toTodo(s: UnifiSite) {
    const saved = await act(() =>
      api.addTodo({
        title: `UniFi: ${s.name}: ${s.issues[0] ?? "check site"}`,
        notes: [
          ...s.issues,
          ...s.problem_devices.map((d) => `${d.name} (${d.model || "device"}${d.ip ? `, ${d.ip}` : ""}) is ${d.status}`),
        ].join("\n"),
        priority: s.health === "down" ? 1 : 2,
      }),
    );
    if (saved) setAdded((a) => new Set(a).add(s.id));
  }

  const sites = snap?.sites ?? [];
  const down = sites.filter((s) => s.health === "down").length;
  const warning = sites.filter((s) => s.health === "warning").length;
  const error = localError ?? loadError ?? snap?.error ?? null;
  const needsKey = snap !== null && !snap.has_key;

  const actions = (
    <>
      {!needsKey && (
        <button className="ghost" disabled={busy} onClick={() => run(unifi.refresh)} title="Check UniFi now" aria-label="Refresh sites">
          {busy ? "…" : "↻"}
        </button>
      )}
      <button className="ghost" onClick={() => setSetup((s) => !s)} title="UniFi settings" aria-label="UniFi settings">
        ⚙
      </button>
    </>
  );

  return (
    <Widget title="Network fleet" actions={actions}>
      {error && (
        <div className="atera-error" role="alert">
          {error}
        </div>
      )}

      {(needsKey || setup) && <UnifiSetup onBusy={run} busy={busy} />}

      {!needsKey && !setup && (
        <>
          {sites.length > 0 && (
            <div className="fleet-summary">
              <span className="fleet-count"><strong>{sites.length}</strong> site{sites.length === 1 ? "" : "s"}</span>
              {down > 0 && <span className="tag health-down">{down} down</span>}
              {warning > 0 && <span className="tag health-warning">{warning} need attention</span>}
              {down === 0 && warning === 0 && <span className="tag health-ok">All healthy</span>}
            </div>
          )}
          {sites.length === 0 ? (
            <p className="empty">{snap?.fetched_at ? "No sites on this UniFi account yet." : "Checking UniFi…"}</p>
          ) : (
            <ul className="fleet-grid">
              {sites.map((s) => {
                const expanded = open === s.id;
                return (
                  <li key={s.id} className={`fleet-tile health-${s.health} ${expanded ? "expanded" : ""}`}>
                    <button className="fleet-head" onClick={() => setOpen(expanded ? null : s.id)} aria-expanded={expanded}>
                      <span className={`health-dot ${s.health}`} title={LABEL[s.health]} aria-label={LABEL[s.health]} />
                      <span className="grow truncate fleet-name" title={s.name}>{s.name}</span>
                    </button>
                    <div className="fleet-stats">
                      <span title="Devices online">
                        <strong>{s.devices_total - s.devices_offline}</strong>/{s.devices_total} devices
                      </span>
                      <span title="Connected clients"><strong>{s.clients}</strong> clients</span>
                      {s.wan_uptime !== null && <span title="WAN uptime"><strong>{fmtPct(s.wan_uptime)}</strong> WAN</span>}
                    </div>
                    {s.issues.length > 0 && <p className="fleet-issue truncate" title={s.issues.join(", ")}>{s.issues.join(" · ")}</p>}
                    {expanded && (
                      <div className="fleet-detail">
                        {s.problem_devices.length > 0 && (
                          <ul className="list dense">
                            {s.problem_devices.map((d) => (
                              <li key={d.id}>
                                <span className={`health-dot ${d.status === "offline" ? "down" : "warning"}`} aria-hidden="true" />
                                <span className="grow truncate">
                                  {d.name}
                                  <span className="sub">{[d.model, d.ip, d.status].filter(Boolean).join(" · ")}</span>
                                </span>
                              </li>
                            ))}
                          </ul>
                        )}
                        <p className="muted small">
                          {[s.console_model, s.console_version && `UniFi OS ${s.console_version}`, s.isp, s.pending_updates > 0 && `${s.pending_updates} update${s.pending_updates === 1 ? "" : "s"} pending`]
                            .filter(Boolean)
                            .join(" · ")}
                        </p>
                        <div className="row">
                          {s.health !== "ok" && (
                            <button className="ghost" disabled={added.has(s.id)} onClick={() => toTodo(s)}>
                              {added.has(s.id) ? "Added" : "+ Todo"}
                            </button>
                          )}
                          <span className="grow" />
                          <button className="ghost" onClick={() => run(() => unifi.setSiteHidden({ id: s.id, name: s.name }, true))} title="Stop showing this site">
                            Hide site
                          </button>
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
            {snap && snap.hidden_sites.length > 0 && (
              <button className="link" onClick={() => setSetup(true)}>
                {snap.hidden_sites.length} hidden
              </button>
            )}
          </div>
        </>
      )}
    </Widget>
  );
}

function fmtPct(n: number): string {
  return `${n >= 99.95 ? 100 : n.toFixed(1)}%`;
}

/** Key entry and hidden sites. Shared by the widget and the Connections settings. */
export function UnifiSetup({ onBusy, busy }: { onBusy?: (fn: () => Promise<unknown>) => Promise<void>; busy?: boolean }) {
  const { snap, reload } = useLiveSnapshot(unifi.fleet, "unifi-changed");
  const [key, setKey] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const run =
    onBusy ??
    (async (fn: () => Promise<unknown>) => {
      setErr(null);
      try {
        await fn();
      } catch (e) {
        setErr(errorText(e));
      } finally {
        reload();
      }
    });

  async function save(e: FormEvent) {
    e.preventDefault();
    await run(() => unifi.setKey(key));
    setKey("");
    reload();
  }

  return (
    <div className="atera-setup">
      <form className="form-row" onSubmit={save}>
        <input
          className="grow"
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder={snap?.has_key ? "Paste a new Site Manager API key to replace it" : "UniFi Site Manager API key"}
          aria-label="UniFi Site Manager API key"
          autoComplete="off"
        />
        <button className="primary" type="submit" disabled={!key.trim() || busy}>
          {snap?.has_key ? "Replace" : "Connect"}
        </button>
        {snap?.has_key && (
          <button type="button" className="ghost" onClick={() => run(() => unifi.setKey("")).then(reload)}>
            Disconnect
          </button>
        )}
      </form>
      {err && <p className="atera-error">{err}</p>}
      <p className="muted small">
        Sign in at unifi.ui.com, open API in the left menu and create a key. It is read-only, covers every console on the
        account, and is stored in {keyStore}. Sites refresh every 5 minutes, and a console, gateway or device going
        offline pops a notification.
      </p>
      {snap && snap.hidden_sites.length > 0 && (
        <div className="stack-tight">
          <span className="muted small">Hidden sites</span>
          <div className="chips">
            {snap.hidden_sites.map((s) => (
              <button key={s.id} onClick={() => run(() => unifi.setSiteHidden(s, false)).then(reload)} title="Show this site again">
                {s.name || s.id} ✕
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
