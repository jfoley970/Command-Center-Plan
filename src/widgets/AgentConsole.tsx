import { useState, type FormEvent } from "react";
import { api, type ProviderId } from "../api";
import { routeCommand } from "../agentRoute";
import { useData } from "../data";
import { formatWhen } from "../time";
import { openUrl } from "../transport";
import Widget from "./Widget";

const STATUS_LABEL = { running: "Running", error: "Error", refused: "Declined", done: "" } as const;

/**
 * Every AI you use in one place: who's connected and working, a box to ask any
 * of them ("grok: …", "@cursor …"), and each one's latest results in place.
 */
export default function AgentConsole({ agentId, setAgentId, onOpenAgents, onOpenSettings }: {
  agentId: number | null;
  setAgentId: (id: number) => void;
  onOpenAgents: () => void;
  onOpenSettings: () => void;
}) {
  const { agents, runs, providers, act } = useData();
  const [input, setInput] = useState("");
  const [shown, setShown] = useState<number | null>(null);
  const [picked, setPicked] = useState<ProviderId | null>(null);

  const current = agents.find((a) => a.id === agentId);
  const providerId: ProviderId = picked ?? current?.provider ?? "claude";
  const provider = providers.find((p) => p.id === providerId);
  const providerAgents = agents.filter((a) => a.provider === providerId);
  const agent = current?.provider === providerId ? current : providerAgents[0];
  const agentRuns = agent ? runs.filter((r) => r.agent_id === agent.id) : [];
  const run = agentRuns.find((r) => r.id === shown) ?? agentRuns[0];

  const working = (id: ProviderId) => runs.some((r) => r.provider === id && r.status === "running");

  function pick(id: ProviderId) {
    setPicked(id);
    setShown(null);
    const first = agents.find((a) => a.provider === id);
    if (first) setAgentId(first.id);
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    const route = routeCommand(input, agents);
    if (route.kind === "provider" && route.provider !== providerId) setPicked(route.provider);
    if (route.kind === "agent") {
      setPicked(null);
      setAgentId(route.agentId);
    }
    setInput("");
    setShown(null);
    const result = await act(() => {
      if (route.kind === "provider") return api.askProvider(route.provider, route.text);
      if (route.kind === "agent") return api.runAgent(route.agentId, route.text);
      return agent ? api.runAgent(agent.id, route.text) : api.askProvider(providerId, route.text);
    });
    if (result) {
      setPicked(null);
      setAgentId(result.agent_id);
    }
  }

  const ready = provider?.connected && !provider.unavailable;
  const name = agent?.name ?? provider?.name ?? "an agent";

  return (
    <Widget
      title="Agents"
      actions={
        <>
          {providerAgents.length > 1 && (
            <select value={agent?.id} onChange={(e) => { setAgentId(Number(e.target.value)); setShown(null); }} aria-label="Agent">
              {providerAgents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>
          )}
          <button className="ghost" onClick={onOpenAgents} title="Make and edit agents">Manage</button>
        </>
      }
    >
      <div className="console">
        <div className="ai-strip" role="tablist" aria-label="AI providers">
          {providers.map((p) => {
            const state = p.unavailable || !p.connected ? "idle" : working(p.id) ? "warning" : "ok";
            const label = p.unavailable ? "not available yet" : !p.connected ? "not connected" : working(p.id) ? "working" : "ready";
            return (
              <button
                key={p.id}
                role="tab"
                aria-selected={p.id === providerId}
                className={p.id === providerId ? "ai-chip active" : "ai-chip"}
                onClick={() => pick(p.id)}
                title={`${p.name}: ${label}`}
              >
                <span className={`status-dot ${state}`} aria-hidden="true" />
                {p.name}
              </button>
            );
          })}
        </div>

        <form onSubmit={submit} className="console-input">
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder={ready ? `Ask ${name}…` : "Start with claude: or chatgpt: to ask another AI"}
            title="Start with a name to pick who answers: claude:, chatgpt:, grok:, @cursor, or an agent's name"
            aria-label="Message for your agents"
          />
          <button className="primary" type="submit" disabled={!input.trim() && !(ready && agent)}>
            {input.trim() ? "Send" : "Run"}
          </button>
        </form>

        <div className="console-output" aria-live="polite">
          {provider?.unavailable ? (
            <p className="empty">{provider.unavailable}</p>
          ) : provider && !provider.connected ? (
            <div className="empty stack-tight">
              <span>{provider.name} isn't connected yet. Paste its API key in Settings &gt; Connections.</span>
              <div><button onClick={onOpenSettings}>Connect {provider.name}</button></div>
            </div>
          ) : !run ? (
            <p className="empty">
              {agent?.description ||
                (provider?.background
                  ? `Make a ${provider.name} agent under Manage and give it a repository to work on.`
                  : `Ask ${provider?.name ?? "an agent"} anything.`)}
            </p>
          ) : (
            <>
              <div className="row between muted small">
                <span>
                  {run.agent_name} · {formatWhen(run.started_at)}
                  {run.input !== "(no extra input)" && ` · “${run.input}”`}
                </span>
                {run.status !== "done" && (
                  <span className={`tag ${run.status === "running" ? "" : "critical"}`}>{STATUS_LABEL[run.status]}</span>
                )}
              </div>
              {run.status === "running" ? (
                <>
                  <div className="thinking" aria-label="Working"><span /><span /><span /></div>
                  {provider?.background && (
                    <p className="muted small">{provider.name} is working in the cloud. You'll get a notification when it finishes, even if you close the app.</p>
                  )}
                </>
              ) : (
                <div className="run-output">{run.output}</div>
              )}
              {run.link && (
                <div>
                  <button className="ghost" onClick={() => openUrl(run.link)}>
                    {run.link.includes("/pull/") ? "Open pull request" : `Open in ${provider?.name ?? "browser"}`}
                  </button>
                </div>
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
    </Widget>
  );
}
