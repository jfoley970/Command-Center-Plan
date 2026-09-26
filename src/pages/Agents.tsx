import { useEffect, useState, type FormEvent } from "react";
import { api, type Agent, type AgentRun, type ModelOption } from "../api";
import { useData } from "../data";
import { formatWhen } from "../time";

type Draft = { id?: number; name: string; description: string; system_prompt: string; model: string };

export default function Agents({ selectedId, onSelect }: { selectedId: number | null; onSelect: (id: number | null) => void }) {
  const { agents, runs, hasKey, act } = useData();
  const [models, setModels] = useState<ModelOption[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [input, setInput] = useState("");
  const [running, setRunning] = useState(false);

  useEffect(() => {
    api.listModels().then(setModels);
  }, []);

  const agent = agents.find((a) => a.id === selectedId) ?? agents[0];
  const agentRuns = agent ? runs.filter((r) => r.agent_id === agent.id) : [];

  function edit(a?: Agent) {
    setDraft(
      a
        ? { id: a.id, name: a.name, description: a.description, system_prompt: a.system_prompt, model: a.model }
        : { name: "", description: "", system_prompt: "", model: models[0]?.id ?? "claude-opus-5" },
    );
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
              Instructions
              <textarea
                rows={8}
                value={draft.system_prompt}
                onChange={(e) => setDraft({ ...draft, system_prompt: e.target.value })}
                placeholder="You are… Your job is…"
                required
              />
            </label>
            <label>
              Model
              <select value={draft.model} onChange={(e) => setDraft({ ...draft, model: e.target.value })}>
                {models.map((m) => (
                  <option key={m.id} value={m.id}>{m.label}</option>
                ))}
              </select>
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
                  <p className="muted">{agent.description}</p>
                </div>
                <div className="row">
                  <button onClick={() => edit(agent)}>Edit</button>
                  <button className="ghost" onClick={() => remove(agent)}>Delete</button>
                </div>
              </div>
              <form className="stack" onSubmit={run}>
                <textarea
                  rows={3}
                  placeholder="Optional: ask something specific. Your todos and reminders are always included."
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                />
                <div className="row">
                  <button className="primary" type="submit" disabled={!hasKey || running}>
                    {running ? "Running…" : "Run"}
                  </button>
                  {!hasKey && <span className="muted">Add your API key in Settings first.</span>}
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
  const label = { running: "Running", done: "Done", error: "Error", refused: "Declined" }[run.status];
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
      {run.status === "done" && (
        <p className="muted small">
          {run.model} · {run.input_tokens.toLocaleString()} in / {run.output_tokens.toLocaleString()} out tokens
        </p>
      )}
    </div>
  );
}
