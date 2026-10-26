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
                  {c.via === "local" && <span className="tag" title="The local AI worked out what this meant">Local AI</span>}
                  <span className="grow truncate">{c.result || c.title}</span>
                  {c.link && <a className="small nowrap" href={c.link}>Open chat</a>}
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
          {c.via === "local" && " · read by Local AI"}
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
  const { providers } = useData();
  const local = providers.find((p) => p.id === "local");
  const [ai, setAi] = useState(true);
  const [aiModel, setAiModel] = useState("");
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
      setAi(snap.ai);
      setAiModel(snap.ai_model);
    }
  }, [snap?.wake, snap?.auto, snap?.ai, snap?.ai_model]);

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
    if (await run(() => pocket.setOptions({ wake, auto, ai, aiModel }))) {
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
        <label className="grow">
          Unclear commands
          <select value={ai ? aiModel || "*" : "off"} onChange={(e) => {
            const v = e.target.value;
            setAi(v !== "off");
            setAiModel(v === "off" || v === "*" ? "" : v);
          }}>
            <option value="*">Local AI reads them (smallest model)</option>
            {local?.models.map((m) => (
              <option key={m.id} value={m.id}>Local AI reads them with {m.label}</option>
            ))}
            {aiModel && !local?.models.some((m) => m.id === aiModel) && <option value={aiModel}>Local AI reads them with {aiModel}</option>}
            <option value="off">Best guess from the rules only</option>
          </select>
        </label>
        <button type="submit" disabled={!wake.trim()}>{saved ? "Saved" : "Save"}</button>
      </form>
      {ai && local && !local.connected && (
        <p className="muted small">Local AI isn't reachable right now ({local.note ?? "no models"}), so unclear commands use the rules' best guess until it is.</p>
      )}
      <p className="muted small">
        Start a recording with the wake phrase, then say what you want: “add a todo to renew the domain”, “remind me at 3 PM
        to call Dana”, “Cursor, fix the login bug” or an agent's name. Say the wake phrase again for another command in
        the same recording. When the simple rules can't tell what you meant, the local AI on the server reads the command
        instead, so it never leaves your server. Recordings that don't start with the wake phrase are ignored and nothing
        from them is kept.
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
