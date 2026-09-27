import { useState, type FormEvent } from "react";
import { api } from "../api";
import { useData } from "../data";
import { formatWhen } from "../time";
import Widget from "./Widget";

/** Ask any agent from the dashboard and read its latest answer in place. */
export default function AgentConsole({ agentId, setAgentId, onOpenAgents }: {
  agentId: number | null;
  setAgentId: (id: number) => void;
  onOpenAgents: () => void;
}) {
  const { agents, runs, hasKey, act } = useData();
  const [input, setInput] = useState("");
  const agent = agents.find((a) => a.id === agentId) ?? agents[0];
  const agentRuns = agent ? runs.filter((r) => r.agent_id === agent.id) : [];
  const [shown, setShown] = useState<number | null>(null);
  const run = agentRuns.find((r) => r.id === shown) ?? agentRuns[0];
  const busy = agentRuns.some((r) => r.status === "running");

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!agent) return;
    const text = input;
    setInput("");
    setShown(null);
    act(() => api.runAgent(agent.id, text));
  }

  return (
    <Widget
      title="Agents"
      actions={
        <>
          {agents.length > 0 && (
            <select value={agent?.id} onChange={(e) => { setAgentId(Number(e.target.value)); setShown(null); }} aria-label="Agent">
              {agents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>
          )}
          <button className="ghost" onClick={onOpenAgents} title="Manage agents">Manage</button>
        </>
      }
    >
      {!agent ? (
        <p className="empty">No agents yet. Create one under Manage.</p>
      ) : (
        <div className="console">
          <form onSubmit={submit} className="console-input">
            <input
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder={hasKey ? `Ask ${agent.name}, or just press Run` : "Add your API key in Settings to run agents"}
              disabled={!hasKey}
              aria-label={`Message for ${agent.name}`}
            />
            <button className="primary" type="submit" disabled={!hasKey || busy}>
              {busy ? "Running…" : "Run"}
            </button>
          </form>

          <div className="console-output" aria-live="polite">
            {!run ? (
              <p className="empty">{agent.description || "No runs yet."}</p>
            ) : (
              <>
                <div className="row between muted small">
                  <span>{formatWhen(run.started_at)}{run.input !== "(no extra input)" && ` · “${run.input}”`}</span>
                  {run.status !== "done" && (
                    <span className={`tag ${run.status === "running" ? "" : "critical"}`}>
                      {{ running: "Running", error: "Error", refused: "Declined", done: "" }[run.status]}
                    </span>
                  )}
                </div>
                {run.status === "running" ? (
                  <div className="thinking" aria-label="Working"><span /><span /><span /></div>
                ) : (
                  <div className="run-output">{run.output}</div>
                )}
              </>
            )}
          </div>

          {agentRuns.length > 1 && (
            <div className="history" aria-label="Earlier runs">
              {agentRuns.slice(0, 8).map((r) => (
                <button
                  key={r.id}
                  className={r.id === run?.id ? "hist active" : "hist"}
                  onClick={() => setShown(r.id)}
                  title={`${formatWhen(r.started_at)}: ${r.input}`}
                >
                  {formatWhen(r.started_at).replace("Today ", "")}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </Widget>
  );
}
