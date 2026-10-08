import { useState, type FormEvent } from "react";
import { api } from "../api";
import { routeCommand } from "../agentRoute";
import { useData } from "../data";
import { minutesFromNow, quick, tomorrowAt9 } from "../quick";

/** Type once, then choose what it becomes. Enter adds a todo. */
export default function CommandBar({ onAsk }: { onAsk: (agentId: number) => void }) {
  const { agents, providers, hasKey, act } = useData();
  const [text, setText] = useState("");
  const [flash, setFlash] = useState<string | null>(null);
  const t = text.trim();
  const planner = agents.find((a) => a.name === "Daily Planner") ?? agents[0];
  const route = routeCommand(t, agents);

  async function run(label: string, fn: () => Promise<unknown>) {
    if (!t) return;
    const ok = await act(fn);
    if (ok !== undefined) {
      setText("");
      setFlash(label);
      setTimeout(() => setFlash(null), 2500);
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    run("Todo added", () => quick.todo(t));
  }

  return (
    <form className="command-bar" onSubmit={submit}>
      <svg className="cb-icon" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
        <path d="M5 12h14M12 5v14" />
      </svg>
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Capture anything. Enter adds a todo, or pick an action →"
        aria-label="Quick capture"
      />
      {flash && <span className="cb-flash" role="status">{flash}</span>}
      <div className="cb-actions">
        <button type="submit" disabled={!t}>Todo</button>
        <button type="button" disabled={!t} onClick={() => run("Reminder set for 30 min", () => quick.remind(t, minutesFromNow(30)))}>
          In 30 min
        </button>
        <button type="button" disabled={!t} onClick={() => run("Reminder set for 9 AM", () => quick.remind(t, tomorrowAt9()))}>
          Tomorrow 9 AM
        </button>
        <button
          type="button"
          className="primary"
          disabled={!t || (!planner && route.kind === "default") || (route.kind === "default" && !hasKey)}
          title="Ask an agent. Start with grok:, chatgpt:, @cursor or an agent's name to pick who answers."
          onClick={async () => {
            // Runs can take a while; clear the bar now and let the agent widget show progress.
            const target =
              route.kind === "agent" ? agents.find((a) => a.id === route.agentId)?.name
              : route.kind === "provider" ? providers.find((p) => p.id === route.provider)?.name
              : planner?.name;
            if (route.kind !== "provider") onAsk(route.kind === "agent" ? route.agentId : planner.id);
            setText("");
            setFlash(`Sent to ${target ?? "agent"}`);
            setTimeout(() => setFlash(null), 2500);
            const run = await act(() =>
              route.kind === "provider" ? api.askProvider(route.provider, route.text)
              : api.runAgent(route.kind === "agent" ? route.agentId : planner.id, route.text),
            );
            if (run) onAsk(run.agent_id);
          }}
        >
          Ask agent
        </button>
      </div>
    </form>
  );
}
