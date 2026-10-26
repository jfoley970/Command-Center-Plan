import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { api, type Agent, type AgentRun, type ProviderId } from "../api";
import { useData } from "../data";
import { formatWhen } from "../time";
import { openUrl } from "../transport";

type Draft = Pick<Agent, "name" | "description" | "system_prompt" | "model" | "provider" | "repo" | "include_context" | "memory"> & { id?: number };

export default function Agents({ selectedId, onSelect }: { selectedId: number | null; onSelect: (id: number | null) => void }) {
  const { agents, providers, act } = useData();
  const [draft, setDraft] = useState<Draft | null>(null);

  const agent = agents.find((a) => a.id === selectedId) ?? agents[0];
  const usable = providers.filter((p) => !p.unavailable);
  const providerOf = (id: ProviderId) => providers.find((p) => p.id === id);
  const draftProvider = draft && providerOf(draft.provider);

  function edit(a?: Agent) {
    setDraft(
      a
        ? { id: a.id, name: a.name, description: a.description, system_prompt: a.system_prompt, model: a.model, provider: a.provider, repo: a.repo, include_context: a.include_context, memory: a.memory }
        : { name: "", description: "", system_prompt: "", model: providerOf("claude")?.models[0]?.id ?? "claude-opus-5", provider: "claude", repo: "", include_context: true, memory: true },
    );
  }

  function setProvider(id: ProviderId) {
    if (!draft) return;
    const p = providerOf(id);
    // Cursor reads the repository itself, so the day's todos aren't sent by default.
    setDraft({ ...draft, provider: id, model: p?.models[0]?.id ?? "", include_context: !p?.background && draft.include_context });
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!draft) return;
    const saved = await act(() => api.saveAgent(draft));
    if (saved) {
      setDraft(null);
      onSelect(saved.id);
    }
  }

  async function remove(a: Agent) {
    if (!confirm(`Delete the agent "${a.name}" and its run history?`)) return;
    await act(() => api.deleteAgent(a.id));
    onSelect(null);
  }

  return (
    <div className="page split">
      <aside className="card agent-list">
        <div className="row between">
          <h2>Agents</h2>
          <button onClick={() => edit()}>New</button>
        </div>
        <ul className="list">
          {agents.map((a) => (
            <li key={a.id} className={a.id === agent?.id ? "active" : ""} onClick={() => onSelect(a.id)}>
              <span className="grow">{a.name}</span>
              <span className="muted small">{providerOf(a.provider)?.name}</span>
            </li>
          ))}
        </ul>
      </aside>

      <div className="grow stack">
        {draft ? (
          <form className="card stack" onSubmit={save}>
            <h2>{draft.id ? "Edit agent" : "New agent"}</h2>
            <label>
              Name
              <input value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} required />
            </label>
            <label>
              What it's for
              <input value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
            </label>
            <label>
              Runs on
              <select value={draft.provider} onChange={(e) => setProvider(e.target.value as ProviderId)}>
                {usable.map((p) => (
                  <option key={p.id} value={p.id}>{p.name}{p.connected ? "" : " (not connected)"}</option>
                ))}
              </select>
            </label>
            {draftProvider?.background && (
              <label>
                GitHub repository
                <input
                  value={draft.repo}
                  onChange={(e) => setDraft({ ...draft, repo: e.target.value })}
                  placeholder="owner/repo"
                  required
                />
              </label>
            )}
            <label>
              {draftProvider?.background ? "Standing instructions (optional)" : "Instructions"}
              <textarea
                rows={8}
                value={draft.system_prompt}
                onChange={(e) => setDraft({ ...draft, system_prompt: e.target.value })}
                placeholder={draftProvider?.background ? "Always add tests. Keep changes small…" : "You are… Your job is…"}
                required={!draftProvider?.background}
              />
            </label>
            {draftProvider && draftProvider.models.length > 1 && (
              <label>
                Model
                <select value={draft.model} onChange={(e) => setDraft({ ...draft, model: e.target.value })}>
                  {draftProvider.models.map((m) => (
                    <option key={m.id} value={m.id}>{m.label}</option>
                  ))}
                </select>
              </label>
            )}
            <label className="row">
              <input
                type="checkbox"
                checked={draft.include_context}
                onChange={(e) => setDraft({ ...draft, include_context: e.target.checked })}
              />
              Include today's open todos and reminders with each run
            </label>
            {!draftProvider?.background && (
              <label className="row">
                <input type="checkbox" checked={draft.memory} onChange={(e) => setDraft({ ...draft, memory: e.target.checked })} />
                Remember the conversation (each message sends the last 10 exchanges)
              </label>
            )}
            <div className="row">
              <button className="primary" type="submit">Save</button>
              <button type="button" onClick={() => setDraft(null)}>Cancel</button>
            </div>
          </form>
        ) : agent ? (
          <AgentChat key={agent.id} agent={agent} onEdit={() => edit(agent)} onDelete={() => remove(agent)} />
        ) : (
          <p className="muted">Create an agent to get started.</p>
        )}
      </div>
    </div>
  );
}

const MIC = (
  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <rect x="9" y="2" width="6" height="12" rx="3" />
    <path d="M5 10a7 7 0 0 0 14 0M12 17v5" />
  </svg>
);

/** One agent's conversation: every run is a message and its reply, oldest first, with a reply box. */
function AgentChat({ agent, onEdit, onDelete }: { agent: Agent; onEdit: () => void; onDelete: () => void }) {
  const { runs, providers, act } = useData();
  const [thread, setThread] = useState<AgentRun[]>([]);
  const [showEarlier, setShowEarlier] = useState(false);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  const provider = providers.find((p) => p.id === agent.provider);

  // Reloads whenever any run changes, which includes Pocket commands and the dashboard's ask box.
  useEffect(() => {
    let live = true;
    api.listRuns(agent.id, 200).then((r) => live && setThread([...r].reverse())).catch(() => {});
    return () => {
      live = false;
    };
  }, [agent.id, runs]);

  const since = agent.chat_since;
  const earlier = since ? thread.filter((r) => r.started_at <= since) : [];
  const current = since ? thread.filter((r) => r.started_at > since) : thread;
  const shown = showEarlier ? thread : current;

  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [shown.length, shown[shown.length - 1]?.status]);

  async function send(e?: FormEvent) {
    e?.preventDefault();
    if (sending || (!input.trim() && !agent.include_context)) return;
    const text = input;
    setInput("");
    setSending(true);
    const done = await act(() => api.runAgent(agent.id, text));
    setSending(false);
    if (!done) setInput(text);
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  async function newChat() {
    if (!confirm(`Start a new chat with ${agent.name}? It won't see the earlier messages, but they stay here.`)) return;
    await act(() => api.clearChat(agent.id));
    setShowEarlier(false);
  }

  return (
    <section className="card chat">
      <div className="row between chat-head">
        <div className="grow">
          <h2>{agent.name}</h2>
          <p className="muted small">
            {provider?.name}
            {agent.repo && ` · ${agent.repo}`}
            {agent.description && ` · ${agent.description}`}
            {agent.memory && !provider?.background && " · remembers this chat"}
          </p>
        </div>
        <div className="row">
          {agent.memory && !provider?.background && current.length > 0 && <button onClick={newChat}>New chat</button>}
          <button onClick={onEdit}>Edit</button>
          <button className="ghost" onClick={onDelete}>Delete</button>
        </div>
      </div>

      <div className="chat-thread" aria-live="polite">
        {earlier.length > 0 && (
          <button className="ghost small chat-earlier" onClick={() => setShowEarlier(!showEarlier)}>
            {showEarlier ? "Hide earlier chats" : `Show earlier chats (${earlier.length})`}
          </button>
        )}
        {shown.length === 0 && <p className="muted chat-empty">{agent.description || `Say something to ${agent.name}.`}</p>}
        {shown.map((r) => {
          const old = !!since && r.started_at <= since;
          return (
            <div key={r.id} className={old ? "chat-turn old" : "chat-turn"}>
              {r.input !== "(no extra input)" && (
                <div className="bubble you">
                  <div className="bubble-text">{r.input}</div>
                  <div className="bubble-meta">
                    {r.source === "pocket" && <span className="via-pocket" title="Said to Pocket">{MIC} Pocket</span>}
                    {formatWhen(r.started_at)}
                  </div>
                </div>
              )}
              <div className={`bubble bot ${r.status === "error" || r.status === "refused" ? "failed" : ""}`}>
                {r.status === "running" ? (
                  <div className="thinking" aria-label="Working"><span /><span /><span /></div>
                ) : (
                  <div className="bubble-text">{r.output}</div>
                )}
                {r.link && (
                  <button className="ghost small" onClick={() => openUrl(r.link)}>
                    {r.link.includes("/pull/") ? "Open pull request" : "Open in browser"}
                  </button>
                )}
                {r.status !== "running" && (
                  <div className="bubble-meta">
                    {r.status === "error" ? "Error · " : r.status === "refused" ? "Declined · " : ""}
                    {r.finished_at ? formatWhen(r.finished_at) : ""}
                  </div>
                )}
              </div>
            </div>
          );
        })}
        <div ref={end} />
      </div>

      <form className="chat-reply" onSubmit={send}>
        <textarea
          rows={2}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKey}
          placeholder={
            provider?.background
              ? "What should it do in the repository?"
              : agent.include_context
                ? `Message ${agent.name}. Your todos and reminders are included.`
                : `Message ${agent.name}…`
          }
          aria-label={`Message for ${agent.name}`}
        />
        <button className="primary" type="submit" disabled={!provider?.connected || sending || (!input.trim() && !agent.include_context)}>
          {sending ? "Sending…" : "Send"}
        </button>
      </form>
      {provider && !provider.connected && <p className="muted small">Add your {provider.name} key in Settings &gt; Connections first.</p>}
    </section>
  );
}
