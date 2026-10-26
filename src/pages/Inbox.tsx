import { useEffect, useState, type FormEvent } from "react";
import { listen, onServer, openUrl } from "../transport";
import { api, type Email, type MailAccount, type MailAction, type Suggestion } from "../api";
import { useData } from "../data";
import { formatWhen, fromLocalInput, toLocalInput } from "../time";

function ago(iso: string | null): string {
  if (!iso) return "never";
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  return formatWhen(iso);
}

export default function Inbox({ selectedId, onSelect, go }: { selectedId: number | null; onSelect: (id: number | null) => void; go: (p: "settings") => void }) {
  const { mailAccounts } = useData();
  const [connecting, setConnecting] = useState(false);

  const account = mailAccounts.find((a) => a.id === selectedId) ?? mailAccounts[0];
  const showConnect = connecting || mailAccounts.length === 0;

  return (
    <div className="page split">
      <aside className="card agent-list">
        <div className="row between">
          <h2>Inboxes</h2>
          {!showConnect && <button onClick={() => setConnecting(true)}>Add</button>}
        </div>
        {mailAccounts.length === 0 && <p className="muted">None connected yet.</p>}
        <ul className="list dense">
          {mailAccounts.map((a) => (
            <li
              key={a.id}
              className={!showConnect && a.id === account?.id ? "active" : ""}
              onClick={() => {
                setConnecting(false);
                onSelect(a.id);
              }}
            >
              <span className="grow truncate" title={a.email}>{a.email}</span>
              {a.pending > 0 && <span className="tag accent" title="Suggestions to review">{a.pending}</span>}
            </li>
          ))}
        </ul>
      </aside>

      <div className="grow stack">
        <PendingMailActions accounts={mailAccounts} />
        {showConnect ? (
          <ConnectCard
            onDone={(a) => {
              setConnecting(false);
              onSelect(a.id);
            }}
            onCancel={mailAccounts.length > 0 ? () => setConnecting(false) : undefined}
          />
        ) : (
          account && <AccountView account={account} onGone={() => onSelect(null)} go={go} />
        )}
      </div>
    </div>
  );
}

/** Sends and deletes wait here; the server runs one only after you approve it. */
function PendingMailActions({ accounts }: { accounts: MailAccount[] }) {
  const { act } = useData();
  const [actions, setActions] = useState<MailAction[]>([]);

  useEffect(() => {
    const load = () => api.listMailActions().then(setActions).catch(() => {});
    load();
    const un = listen("mail-actions-changed", load);
    return () => void un.then((f) => f());
  }, []);

  const pending = actions.filter((a) => a.status === "pending");
  if (pending.length === 0) return null;
  const email = (id: number) => accounts.find((a) => a.id === id)?.email ?? "an inbox";
  return (
    <section className="card stack">
      <h2>Waiting for your approval</h2>
      <p className="muted small">Command Center never sends or deletes mail until you approve it here.</p>
      <ul className="list">
        {pending.map((a) => (
          <li key={a.id} className="row between">
            <span className="grow">
              {a.summary}
              <span className="sub">
                {email(a.account_id)} · {a.method} {a.path}
                {a.requested_by ? ` · asked by ${a.requested_by}` : ""}
              </span>
            </span>
            <button className="primary" onClick={() => act(() => api.approveMailAction(a.id))}>Approve</button>
            <button className="ghost" onClick={() => act(() => api.rejectMailAction(a.id))}>Reject</button>
          </li>
        ))}
      </ul>
    </section>
  );
}

// Keep in step with SCOPES in crates/cc-core/src/mail.rs.
const SCOPES = ["offline_access", "User.Read", "Mail.ReadWrite", "Mail.Send", "MailboxSettings.ReadWrite", "Calendars.ReadWrite", "Contacts.ReadWrite"];

function ConnectCard({ onDone, onCancel }: { onDone: (a: MailAccount) => void; onCancel?: () => void }) {
  const { act } = useData();
  const [clientId, setClientId] = useState("");
  const [tenantId, setTenantId] = useState("");
  const [waiting, setWaiting] = useState(false);
  const [missing, setMissing] = useState(false);
  // On the server, Microsoft sign-in uses a code you enter at a link, from any device.
  const [prompt, setPrompt] = useState<{ user_code: string; verification_uri: string } | null>(null);

  useEffect(() => {
    const un = listen<{ user_code: string; verification_uri: string }>("mail-sign-in", (e) => setPrompt(e.payload));
    return () => void un.then((f) => f());
  }, []);

  // The event only arrives over the WebSocket, so also ask for the code directly.
  useEffect(() => {
    if (!waiting || !onServer || prompt) return;
    const t = setInterval(() => api.mailSignInPrompt().then((p) => p && setPrompt(p)).catch(() => {}), 1500);
    return () => clearInterval(t);
  }, [waiting, prompt]);

  useEffect(() => {
    api.getMailSetup().then((s) => {
      setClientId((c) => c || s.client_id);
      setTenantId((t) => t || s.tenant_id);
    });
  }, []);

  async function connect(e: FormEvent) {
    e.preventDefault();
    if (!clientId.trim() || !tenantId.trim()) {
      setMissing(true);
      return;
    }
    setMissing(false);
    setWaiting(true);
    const account = await act(() => api.connectMicrosoft(clientId, tenantId));
    setWaiting(false);
    setPrompt(null);
    if (account) onDone(account);
  }

  return (
    <form className="card stack" onSubmit={connect}>
      <h2>Connect a Microsoft 365 inbox</h2>
      <p className="muted">
        Command Center reads your inbox, summarizes it, and suggests todos and reminders for you to approve. Signing in grants it
        full control of this mailbox, calendar and contacts. Reading, flagging, moving and archiving can happen on their own;
        it never sends or deletes mail until you approve each one. {onServer ? "Your sign-in stays encrypted on the Command Center server." : "Your sign-in stays in your operating system's keychain."}
      </p>
      <label>
        Application (client) ID
        <input value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" autoComplete="off" />
      </label>
      <label>
        Directory (tenant) ID
        <input value={tenantId} onChange={(e) => setTenantId(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" autoComplete="off" />
      </label>
      <p className="muted small">
        Both are on the Overview page of the Command Center app registration at entra.microsoft.com. If the app is registered in a
        different tenant than this mailbox (for example because GoDaddy manages your 365 tenant), make it multitenant and enter
        organizations (work accounts in any tenant) or common (work or personal) as the tenant ID instead.
      </p>
      <p className="muted small">
        In the app registration, under API permissions &gt; Microsoft Graph &gt; Delegated, add: {SCOPES.join(", ")}.
      </p>
      {onServer && (
        <p className="muted small">In the app registration, turn on Authentication &gt; Allow public client flows, so the server can sign in with a code.</p>
      )}
      {missing && <p className="notice bad" role="alert">Enter both IDs from the app registration first.</p>}
      {prompt && (
        <div className="card stack">
          <p>
            Open <a onClick={() => openUrl(prompt.verification_uri)}>{prompt.verification_uri}</a> on any device and enter this code:
          </p>
          <p className="device-code">{prompt.user_code}</p>
        </div>
      )}
      <div className="row">
        <button className="primary" type="submit" disabled={waiting}>
          {waiting ? (onServer ? "Waiting for Microsoft sign-in…" : "Finish signing in in your browser…") : "Sign in with Microsoft"}
        </button>
        {onCancel && !waiting && <button type="button" onClick={onCancel}>Cancel</button>}
      </div>
    </form>
  );
}

function AccountView({ account, onGone, go }: { account: MailAccount; onGone: () => void; go: (p: "settings") => void }) {
  const { suggestions, flagLinks, todos, hasKey, act } = useData();
  const [emails, setEmails] = useState<Email[]>([]);
  const [syncing, setSyncing] = useState(false);

  useEffect(() => {
    api.listEmails(account.id).then(setEmails).catch(() => setEmails([]));
  }, [account.id, account.last_sync_at]);

  async function sync() {
    setSyncing(true);
    await act(() => api.syncMail(account.id));
    setSyncing(false);
  }

  async function disconnect() {
    if (!confirm(`Disconnect ${account.email}? Its stored mail and pending suggestions are removed. Todos you already added stay.`)) return;
    await act(() => api.disconnectMail(account.id));
    onGone();
  }

  const mine = suggestions.filter((s) => s.account_id === account.id);
  const flags = flagLinks.filter((f) => f.account_id === account.id && f.flagged);
  const todoDone = (id: number | null) => todos.find((t) => t.id === id)?.done;

  return (
    <>
      <section className="card stack">
        <div className="row between">
          <div>
            <h1 className="truncate">{account.display_name || account.email}</h1>
            <p className="muted small">
              {account.email} · {account.unread} unread · synced {ago(account.last_sync_at)}
            </p>
          </div>
          <div className="row">
            <button onClick={sync} disabled={syncing}>{syncing ? "Syncing…" : "Sync now"}</button>
            <button className="ghost" onClick={disconnect}>Disconnect</button>
          </div>
        </div>
        {account.last_error && <div className="notice bad"><span className="grow">{account.last_error}</span></div>}
      </section>

      <section className="card stack">
        <h2>Flagged in Outlook {flags.length > 0 && <span className="tag accent">{flags.length}</span>}</h2>
        {flags.length === 0 && <p className="muted">Flag an email in Outlook and it becomes a todo here on the next sync. Clearing or completing the flag checks the todo off.</p>}
        <ul className="list">
          {flags.map((f) => (
            <li key={f.web_link || f.subject} className={todoDone(f.todo_id) ? "done" : ""}>
              <span className="grow truncate">{f.subject || "(no subject)"}<span className="muted small"> · {f.sender}</span></span>
              <span className="tag">{f.todo_id == null ? "Todo deleted" : todoDone(f.todo_id) ? "Done" : "In todos"}</span>
              {f.web_link && <button className="ghost" onClick={() => openUrl(f.web_link)} title="Open in Outlook" aria-label="Open in Outlook">↗</button>}
            </li>
          ))}
        </ul>
      </section>

      {!hasKey ? (
        <section className="card row between">
          <p className="muted">Optional: add a Claude API key to get an inbox summary and suggested tasks. This is billed by Anthropic per use.</p>
          <button onClick={() => go("settings")}>Open Settings</button>
        </section>
      ) : (
      <>
      <section className="card stack">
        <div className="row between">
          <h2>Summary</h2>
          {account.summary_at && <span className="muted small">Updated {ago(account.summary_at)}</span>}
        </div>
        {account.summary ? (
          <p className="summary">{account.summary}</p>
        ) : (
          <p className="muted">{syncing || !account.last_sync_at ? "Reading your inbox…" : "No summary yet. It appears after the next sync with new mail."}</p>
        )}
      </section>

      <section className="card stack">
        <h2>Suggested from email {mine.length > 0 && <span className="tag accent">{mine.length}</span>}</h2>
        {mine.length === 0 && <p className="muted">Nothing to review. New asks and deadlines found in your mail show up here.</p>}
        <ul className="list">
          {mine.map((s) => (
            <SuggestionRow key={s.id} s={s} />
          ))}
        </ul>
      </section>
      </>
      )}

      <section className="card stack">
        <h2>Recent mail</h2>
        {emails.length === 0 && <p className="muted">No messages synced yet.</p>}
        <ul className="list">
          {emails.map((e) => (
            <li key={e.id} className={e.is_read ? "email" : "email unread"}>
              <div className="grow">
                <div className="row between">
                  <span className="truncate from">{e.from_name || e.from_addr}</span>
                  <span className="muted small nowrap">{formatWhen(e.received_at)}</span>
                </div>
                <div className="truncate subject">{e.subject || "(no subject)"}</div>
                <div className="truncate muted small">{e.preview}</div>
              </div>
              {e.web_link && (
                <button className="ghost" onClick={() => openUrl(e.web_link)} title="Open in Outlook" aria-label="Open in Outlook">↗</button>
              )}
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}

function SuggestionRow({ s }: { s: Suggestion }) {
  const { projects, act, setError } = useData();
  const [kind, setKind] = useState(s.kind);
  const [title, setTitle] = useState(s.title);
  const [when, setWhen] = useState(s.due_at ? toLocalInput(new Date(s.due_at)) : "");
  const [projectId, setProjectId] = useState(s.project_id ? String(s.project_id) : "");

  function accept() {
    if (kind === "reminder" && !when) {
      setError("Pick a time for the reminder first.");
      return;
    }
    act(() =>
      api.acceptSuggestion({
        id: s.id,
        kind,
        title,
        due_at: when ? fromLocalInput(when) : null,
        project_id: projectId ? Number(projectId) : null,
      }),
    );
  }

  return (
    <li className="suggestion">
      <div className="grow stack tight">
        <div className="form-row">
          <select value={kind} onChange={(e) => setKind(e.target.value as Suggestion["kind"])} aria-label="Type">
            <option value="todo">Todo</option>
            <option value="reminder">Reminder</option>
          </select>
          <input className="grow" value={title} onChange={(e) => setTitle(e.target.value)} aria-label="Title" />
          <input type="datetime-local" value={when} onChange={(e) => setWhen(e.target.value)} aria-label={kind === "reminder" ? "Remind at" : "Due"} />
          <select value={projectId} onChange={(e) => setProjectId(e.target.value)} aria-label="Project">
            <option value="">No project</option>
            {projects.filter((p) => !p.archived).map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </div>
        <p className="muted small">
          {s.notes}
          {s.email_subject && (
            <>
              {" "}
              {s.email_link ? (
                <a onClick={() => openUrl(s.email_link!)}>From {s.email_from}: “{s.email_subject}”</a>
              ) : (
                <>From {s.email_from}: “{s.email_subject}”</>
              )}
            </>
          )}
        </p>
      </div>
      <div className="row">
        <button className="primary" onClick={accept}>Add</button>
        <button className="ghost" onClick={() => act(() => api.dismissSuggestion(s.id))}>Dismiss</button>
      </div>
    </li>
  );
}
