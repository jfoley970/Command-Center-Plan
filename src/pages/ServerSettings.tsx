// Desktop only: point this app at a Command Center server, so it shows the same
// data and connectors as the web app, and move connectors set up on this
// computer to the server.
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorText, type ConnectorSummary, type ImportResult } from "../api";
import { listen, type DesktopServer } from "../transport";

type Sides = { local: ConnectorSummary[]; server: ConnectorSummary[] };

const desktop = {
  get: () => invoke<DesktopServer>("desktop_server_get"),
  save: (url: string, username: string, token: string) => invoke<void>("desktop_server_save", { url, username, token }),
  connectors: () => invoke<Sides>("desktop_connectors"),
  transfer: (ids: string[]) => invoke<ImportResult[]>("desktop_connectors_transfer", { ids }),
  use: (server: boolean) => invoke<void>("desktop_server_use", { server }),
};

export default function ServerSettings() {
  const [s, setS] = useState<DesktopServer | null>(null);
  const [url, setUrl] = useState("");
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [sides, setSides] = useState<Sides | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [results, setResults] = useState<ImportResult[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const next = await desktop.get();
    setS(next);
    return next;
  }, []);

  const loadSides = useCallback(async () => {
    try {
      const got = await desktop.connectors();
      setSides(got);
      // Start with what is set up here and missing on the server.
      setPicked(new Set(got.local.filter((c) => c.configured && !got.server.find((x) => x.id === c.id)?.configured).map((c) => c.id)));
    } catch (e) {
      setSides(null);
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    load().then((next) => {
      setUrl(next.url);
      setUsername(next.username);
      if (next.url && next.hasToken) void loadSides();
    });
    const un = listen("server-status", () => void load());
    return () => void un.then((f) => f());
  }, [load, loadSides]);

  async function run<T>(label: string, fn: () => Promise<T>): Promise<T | undefined> {
    setBusy(label);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(errorText(e));
      return undefined;
    } finally {
      setBusy(null);
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    const ok = await run("save", async () => {
      await desktop.save(url, username, token);
      return true;
    });
    if (!ok) return;
    setToken("");
    await load();
    await run("test", loadSides);
  }

  async function transfer() {
    const r = await run("transfer", () => desktop.transfer([...picked]));
    if (r) {
      setResults(r);
      await loadSides();
    }
  }

  if (!s) return null;
  const onServer = s.mode === "server";
  const name = (id: string) => sides?.local.find((c) => c.id === id)?.name ?? id;

  return (
    <section className="card stack">
      <div>
        <h2>Command Center server</h2>
        <p className="muted small">
          {onServer ? (
            <>
              <span className={`status-dot ${s.connected ? "ok" : "error"}`} aria-hidden="true" /> This app runs on{" "}
              <strong>{s.url}</strong>, the same data and connectors as the web app.{" "}
              {!s.connected && (s.error ?? "Connecting…")}
            </>
          ) : (
            "This app runs on this computer's own data. Point it at your server to make it the same app as the web version, with the same todos, reminders and connectors."
          )}
        </p>
      </div>

      <form className="stack-tight" onSubmit={save}>
        <label>
          Server address
          <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://cc.example.com" autoComplete="off" />
        </label>
        <div className="form-row">
          <label className="grow">
            Authentik username
            <input value={username} onChange={(e) => setUsername(e.target.value)} placeholder="james" autoComplete="off" />
          </label>
          <label className="grow">
            App password
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder={s.hasToken ? "Saved. Paste a new one to replace it" : "From Authentik > Settings > Tokens and App passwords"}
              autoComplete="off"
            />
          </label>
        </div>
        <p className="muted small">
          Make an app password for this computer in Authentik so it can sign in without a browser, and revoke it there if the computer is lost. It is
          stored in your operating system's keychain. Leave the username empty to use the server's API token instead.
        </p>
        <div className="row">
          <button className="primary" type="submit" disabled={!!busy || !url.trim() || (!token.trim() && !s.hasToken)}>
            {busy === "save" || busy === "test" ? "Checking…" : "Save and test"}
          </button>
          {onServer ? (
            <button type="button" className="ghost" disabled={!!busy} onClick={() => run("switch", () => desktop.use(false))}>
              Use this computer's data instead
            </button>
          ) : (
            <button type="button" disabled={!!busy || !sides} onClick={() => run("switch", () => desktop.use(true))} title={sides ? "" : "Save and test first"}>
              {busy === "switch" ? "Switching…" : "Switch this app to the server"}
            </button>
          )}
        </div>
      </form>

      {sides && (
        <div className="stack-tight">
          <h3 className="conn-group-title">Move connectors to the server</h3>
          <p className="muted small">
            Copies keys, hidden customers and sites, and signed-in Microsoft 365 inboxes from this computer straight to the server, so the web app
            and this app share them. Keys go from your keychain to the server's encrypted store over HTTPS and never pass through this page.
          </p>
          <ul className="conn-list">
            {sides.local.map((c) => {
              const there = sides.server.find((x) => x.id === c.id);
              return (
                <li key={c.id} className="conn-row">
                  <label className="conn-head">
                    <input
                      type="checkbox"
                      disabled={!c.configured}
                      checked={picked.has(c.id)}
                      onChange={(e) => {
                        const next = new Set(picked);
                        if (e.target.checked) next.add(c.id);
                        else next.delete(c.id);
                        setPicked(next);
                      }}
                    />
                    <span className="grow conn-text">
                      <span className="conn-name">{c.name}</span>
                      <span className="sub truncate">This computer: {c.detail}</span>
                    </span>
                    <span className={`tag ${there?.configured ? "" : "bad"}`}>{there?.configured ? `Server: ${there.detail}` : "Not on the server"}</span>
                  </label>
                </li>
              );
            })}
          </ul>
          <div className="row">
            <button className="primary" disabled={!!busy || picked.size === 0} onClick={transfer}>
              {busy === "transfer" ? "Copying…" : `Copy ${picked.size || ""} to the server`}
            </button>
          </div>
          {results && (
            <ul className="list plain small">
              {results.map((r) => (
                <li key={r.id}>
                  <span className={`status-dot ${r.ok ? "ok" : "error"}`} aria-hidden="true" /> {name(r.id)}: {r.detail}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {error && <div className="notice bad small">{error}</div>}
    </section>
  );
}
