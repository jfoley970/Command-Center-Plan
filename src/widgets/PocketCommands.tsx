import { useEffect, useState, type FormEvent } from "react";
import { errorText } from "../api";
import { ago } from "../atera";
import { useData } from "../data";
import { useLiveSnapshot } from "../live";
import { AUTO_LABEL, pocket, type PocketAuto, type PocketCommand, type PocketKind } from "../pocket";
import { formatWhen, fromLocalInput, toLocalInput } from "../time";
import { keyStore, onServer } from "../transport";
import Widget from "./Widget";
import "./pocket.css";

export function usePocket() {
  return useLiveSnapshot(pocket.status, "pocket-changed");
}

/** Voice commands from Pocket recordings: what ran, and what's waiting for a tap. */
export default function PocketCommands() {
  const { snap, error: loadError, reload } = usePocket();
  const [setup, setSetup] = useState(false);
  const [busy, setBusy] = useState(false);
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

  const error = localError ?? loadError ?? snap?.error ?? null;
  const connected = !!snap && (snap.has_key || snap.has_webhook_secret);
  const commands = (snap?.commands ?? []).filter((c) => c.status !== "dismissed");
  const pending = commands.filter((c) => c.status === "pending" || c.status === "error");
  const done = commands.filter((c) => c.status === "done").slice(0, 8);

  const actions = (
    <>
      {snap?.has_key && (
        <button className="ghost" disabled={busy} onClick={() => run(pocket.refresh)} title="Check Pocket now" aria-label="Check Pocket now">
          {busy ? "…" : "↻"}
        </button>
      )}
      <button className="ghost" onClick={() => setSetup((s) => !s)} title="Pocket settings" aria-label="Pocket settings">
        ⚙
      </button>
    </>
  );

  return (
    <Widget title="Pocket commands" actions={actions}>
      {error && <div className="atera-error" role="alert">{error}</div>}
      {snap && (!connected || setup) ? (
        <PocketSetup />
      ) : (
        <>
          {snap && (
            <p className="muted small pocket-hint">
              Press record and say “{capital(snap.wake)}, …”. {snap.checked_at && `Checked ${ago(snap.checked_at)}.`}
            </p>
          )}
          {pending.length === 0 && done.length === 0 && (
            <p className="empty">
              No commands yet. Try “{capital(snap?.wake ?? "Command Center")}, remind me at 3 PM to call Dana.”
            </p>
          )}
          {pending.length > 0 && (
            <ul className="list pocket-list">
              {pending.map((c) => (
                <PendingRow key={c.id} c={c} onDone={reload} />
              ))}
            </ul>
          )}
          {done.length > 0 && (
            <ul className="list dense pocket-list">
              {done.map((c) => (
                <li key={c.id} title={`Heard: “${c.heard}”`}>
                  <span className={`tag kind-${c.kind}`}>{KIND_LABEL[c.kind]}</span>
                  <span className="grow truncate">{c.result || c.title}</span>
                  <span className="muted small nowrap">{ago(c.created_at)}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </Widget>
  );
}

const KIND_LABEL: Record<PocketKind, string> = { todo: "Todo", reminder: "Reminder", agent: "Agent" };

function capital(s: string): string {
  return s.replace(/\b\w/g, (c) => c.toUpperCase());
}

function PendingRow({ c, onDone }: { c: PocketCommand; onDone: () => void }) {
  const { act } = useData();
  const [kind, setKind] = useState<PocketKind>(c.kind);
  const [title, setTitle] = useState(c.title);
  const [when, setWhen] = useState(c.due_at ? toLocalInput(new Date(c.due_at)) : "");

  async function approve() {
    await act(() => pocket.run(c.id, { kind, title, due_at: when ? fromLocalInput(when) : null }));
    onDone();
  }

  return (
    <li className="suggestion">
      <div className="grow stack tight">
        <div className="form-row">
          <select value={kind} onChange={(e) => setKind(e.target.value as PocketKind)} aria-label="Type">
            <option value="todo">Todo</option>
            <option value="reminder">Reminder</option>
            {c.target && <option value="agent">Ask {c.target_name}</option>}
          </select>
          <input className="grow" value={title} onChange={(e) => setTitle(e.target.value)} aria-label="Title" />
          {kind === "reminder" && (
            <input type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} aria-label="Remind at" />
          )}
        </div>
        <p className="muted small">
          {c.status === "error" && <span className="pocket-failed">{c.result} </span>}
          Heard “{c.heard}”{c.recorded_at && ` · ${formatWhen(c.recorded_at)}`}
        </p>
      </div>
      <div className="row">
        <button className="primary" onClick={approve} disabled={!title.trim() || (kind === "reminder" && !when)}>
          {c.status === "error" ? "Retry" : kind === "agent" ? "Send" : "Add"}
        </button>
        <button className="ghost" onClick={() => act(() => pocket.dismiss(c.id)).then(onDone)}>Dismiss</button>
      </div>
    </li>
  );
}

/** Key, wake phrase, auto-run and (on the server) webhook. Shared with Settings > Connections. */
export function PocketSetup() {
  const { snap, reload } = usePocket();
  const [key, setKey] = useState("");
  const [secret, setSecret] = useState("");
  const [wake, setWake] = useState("");
  const [auto, setAuto] = useState<PocketAuto>("tasks");
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (snap) {
      setWake((w) => w || snap.wake);
      setAuto(snap.auto);
    }
  }, [snap?.wake, snap?.auto]);

  async function run(fn: () => Promise<unknown>) {
    setErr(null);
    try {
      await fn();
      return true;
    } catch (e) {
      setErr(errorText(e));
      return false;
    } finally {
      reload();
    }
  }

  async function saveKey(e: FormEvent) {
    e.preventDefault();
    if (await run(() => pocket.setKey(key))) setKey("");
  }

  async function saveSecret(e: FormEvent) {
    e.preventDefault();
    if (await run(() => pocket.setWebhookSecret(secret))) setSecret("");
  }

  async function saveOptions(e: FormEvent) {
    e.preventDefault();
    if (await run(() => pocket.setOptions(wake, auto))) {
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    }
  }

  const hookUrl = `${window.location.origin}/hooks/pocket`;

  return (
    <div className="stack-tight pocket-setup">
      <form className="form-row" onSubmit={saveKey}>
        <input
          className="grow"
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder={snap?.has_key ? "Paste a new Pocket API key to replace it" : "Pocket API key (pk_…)"}
          aria-label="Pocket API key"
          autoComplete="off"
        />
        <button className="primary" type="submit" disabled={!key.trim()}>{snap?.has_key ? "Replace" : "Connect"}</button>
        {snap?.has_key && <button type="button" className="ghost" onClick={() => run(() => pocket.setKey(""))}>Disconnect</button>}
      </form>
      <p className="muted small">
        In the Pocket app go to Settings, Developer, API Keys. The key is stored in {keyStore}. Command Center checks for
        new recordings every 2 minutes and only reads ones made after you connect.
      </p>

      <form className="form-row" onSubmit={saveOptions}>
        <label className="grow">
          Wake phrase
          <input value={wake} onChange={(e) => setWake(e.target.value)} placeholder="command center" />
        </label>
        <label className="grow">
          Run automatically
          <select value={auto} onChange={(e) => setAuto(e.target.value as PocketAuto)}>
            {(Object.keys(AUTO_LABEL) as PocketAuto[]).map((a) => (
              <option key={a} value={a}>{AUTO_LABEL[a]}</option>
            ))}
          </select>
        </label>
        <button type="submit" disabled={!wake.trim()}>{saved ? "Saved" : "Save"}</button>
      </form>
      <p className="muted small">
        Start a recording with the wake phrase, then say what you want: “add a todo to renew the domain”, “remind me at 3 PM
        to call Dana”, “Cursor, fix the login bug” or an agent's name. Say the wake phrase again for another command in
        the same recording. Recordings that don't start with it are ignored and nothing from them is kept.
      </p>

      {onServer && (
        <>
          <form className="form-row" onSubmit={saveSecret}>
            <input
              className="grow"
              type="password"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
              placeholder={snap?.has_webhook_secret ? "Paste a new signing secret to replace it" : "Webhook signing secret (optional)"}
              aria-label="Pocket webhook signing secret"
              autoComplete="off"
            />
            <button type="submit" disabled={!secret.trim()}>{snap?.has_webhook_secret ? "Replace" : "Save"}</button>
            {snap?.has_webhook_secret && (
              <button type="button" className="ghost" onClick={() => run(() => pocket.setWebhookSecret(""))}>Turn off</button>
            )}
          </form>
          <p className="muted small">
            For instant commands, add a webhook in the Pocket app's integrations pointing to <code>{hookUrl}</code> and paste
            the signing secret it shows. Requests without a valid signature are refused.
          </p>
        </>
      )}
      {err && <p className="atera-error">{err}</p>}
    </div>
  );
}
