import { useState, type FormEvent } from "react";
import { api, type Agent, type AgentRun, type ProviderId } from "../api";
import { useData } from "../data";
import { formatWhen } from "../time";
import { openUrl } from "../transport";

type Draft = Pick<Agent, "name" | "description" | "system_prompt" | "model" | "provider" | "repo" | "include_context"> & { id?: number };

export default function Agents({ selectedId, onSelect }: { selectedId: number | null; onSelect: (id: number | null) => void }) {
  const { agents, runs, providers, act } = useData();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [input, setInput] = useState("");
  const [running, setRunning] = useState(false);

  const agent = agents.find((a) => a.id === selectedId) ?? agents[0];
  const agentRuns = agent ? runs.filter((r) => r.agent_id === agent.id) : [];
  const usable = providers.filter((p) => !p.unavailable);
  const providerOf = (id: ProviderId) => providers.find((p) => p.id === id);
  const draftProvider = draft && providerOf(draft.provider);
  const agentProvider = agent && providerOf(agent.provider);

  function edit(a?: Agent) {
    setDraft(
      a
        ? { id: a.id, name: a.name, description: a.description, system_prompt: a.system_prompt, model: a.model, provider: a.provider, repo: a.repo, include_context: a.include_context }
        : { name: "", description: "", system_prompt: "", model: providerOf("claude")?.models[0]?.id ?? "claude-opus-5", provider: "claude", repo: "", include_context: true },
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

  async function run(e: FormEvent) {
    e.preventDefault();
    if (!agent) return;
    setRunning(true);
    const done = await act(() => api.runAgent(agent.id, input));
    setRunning(false);
    if (done) setInput("");
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
            <div className="row">
              <button className="primary" type="submit">Save</button>
              <button type="button" onClick={() => setDraft(null)}>Cancel</button>
            </div>
          </form>
        ) : agent ? (
          <>
            <section className="card stack">
              <div className="row between">
                <div>
                  <h2>{agent.name}</h2>
                  <p className="muted">
                    {agentProvider?.name}
                    {agent.repo && ` · ${agent.repo}`}
                    {agent.description && ` · ${agent.description}`}
                  </p>
                </div>
                <div className="row">
                  <button onClick={() => edit(agent)}>Edit</button>
                  <button className="ghost" onClick={() => remove(agent)}>Delete</button>
                </div>
              </div>
              <form className="stack" onSubmit={run}>
                <textarea
                  rows={3}
                  placeholder={
                    agentProvider?.background
                      ? "What should it do in the repository?"
                      : agent.include_context
                        ? "Optional: ask something specific. Your todos and reminders are included."
                        : "Ask something."
                  }
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                />
                <div className="row">
                  <button className="primary" type="submit" disabled={!agentProvider?.connected || running}>
                    {running ? "Running…" : "Run"}
                  </button>
                  {agentProvider && !agentProvider.connected && (
                    <span className="muted">Add your {agentProvider.name} key in Settings &gt; Connections first.</span>
                  )}
                </div>
              </form>
            </section>
            {agentRuns.map((r) => (
              <section className="card" key={r.id}>
                <RunCard run={r} />
              </section>
            ))}
          </>
        ) : (
          <p className="muted">Create an agent to get started.</p>
        )}
      </div>
    </div>
  );
}

export function RunCard({ run }: { run: AgentRun }) {
  const label = { running: "Running", done: "Done", error: "Error", refused: "Declined", stopped: "Stopped" }[run.status];
  return (
    <div className="run">
      <div className="row between muted small">
        <span>
          {run.agent_name} · {formatWhen(run.started_at)}
        </span>
        <span className={`tag ${run.status === "error" || run.status === "refused" ? "bad" : ""}`}>{label}</span>
      </div>
      {run.input && run.input !== "(no extra input)" && <p className="run-input">{run.input}</p>}
      {run.status === "running" ? <p className="muted">Working…</p> : <div className="run-output">{run.output}</div>}
      {run.link && (
        <button className="ghost" onClick={() => openUrl(run.link)}>
          {run.link.includes("/pull/") ? "Open pull request" : "Open in browser"}
        </button>
      )}
      {run.status === "done" && run.input_tokens + run.output_tokens > 0 && (
        <p className="muted small">
          {run.model} · {run.input_tokens.toLocaleString()} in / {run.output_tokens.toLocaleString()} out tokens
        </p>
      )}
    </div>
  );
}
