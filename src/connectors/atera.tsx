import { useState, type FormEvent } from "react";
import { errorText } from "../api";
import { ago, atera } from "../atera";
import { useLiveSnapshot } from "../live";
import { registerConnector } from "./registry";

function useAtera() {
  return useLiveSnapshot(atera.alerts, "atera-changed");
}

function AteraPanel() {
  const { snap, reload } = useAtera();
  const [key, setKey] = useState("");
  const [err, setErr] = useState<string | null>(null);

  async function run(fn: () => Promise<unknown>) {
    setErr(null);
    try {
      await fn();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      reload();
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    await run(() => atera.setKey(key));
    setKey("");
  }

  return (
    <div className="stack-tight">
      <form className="form-row" onSubmit={save}>
        <input
          className="grow"
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder={snap?.has_key ? "Paste a new Atera API key to replace it" : "Atera API key"}
          aria-label="Atera API key"
          autoComplete="off"
        />
        <button className="primary" type="submit" disabled={!key.trim()}>{snap?.has_key ? "Replace" : "Connect"}</button>
        {snap?.has_key && (
          <button type="button" className="ghost" onClick={() => run(() => atera.setKey(""))}>
            Disconnect
          </button>
        )}
      </form>
      {err && <p className="atera-error">{err}</p>}
      <p className="muted small">
        In Atera go to Admin, Data management, API and create a token with read access to alerts. Hidden customers are
        managed from the widget's settings.
      </p>
    </div>
  );
}

registerConnector({
  id: "atera",
  name: "Atera",
  group: "Monitoring",
  description: "Open RMM alerts on the dashboard, with notifications for new critical ones.",
  useStatus: () => {
    const { snap, error } = useAtera();
    if (error) return { state: "attention", detail: error };
    if (!snap?.has_key) return { state: "off", detail: "Add an API key to see alerts" };
    if (snap.error) return { state: "attention", detail: snap.error };
    return {
      state: "connected",
      detail: `${snap.alerts.length} open alert${snap.alerts.length === 1 ? "" : "s"}${snap.fetched_at ? ` · updated ${ago(snap.fetched_at)}` : ""}`,
    };
  },
  Panel: AteraPanel,
});
