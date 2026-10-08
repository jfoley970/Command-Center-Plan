// Turns a typed (or, later, spoken) command into "who should handle this".
// "grok: what's new on X", "@cursor fix the login bug" and "Daily Planner: ..."
// all name their target up front; anything else goes to the default.
import type { Agent, ProviderId } from "./api";

export type Route =
  | { kind: "provider"; provider: ProviderId; text: string }
  | { kind: "agent"; agentId: number; text: string }
  | { kind: "default"; text: string };

const ALIASES: Record<string, ProviderId> = {
  claude: "claude",
  chatgpt: "chatgpt",
  gpt: "chatgpt",
  openai: "chatgpt",
  grok: "grok",
  cursor: "cursor",
  copilot: "copilot",
};

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

export function routeCommand(input: string, agents: Pick<Agent, "id" | "name">[]): Route {
  const text = input.trim();
  // "@name rest", "name: rest" or "name, rest". Agent names can have spaces, so
  // the colon/comma form takes everything before it.
  const m = text.match(/^@(\S+)\s+([\s\S]*)$/) ?? text.match(/^([^:,]{1,40})[:,]\s*([\s\S]*)$/);
  if (m) {
    const target = norm(m[1]);
    const rest = m[2].trim();
    const agent = agents.find((a) => norm(a.name) === target);
    if (agent) return { kind: "agent", agentId: agent.id, text: rest };
    const provider = ALIASES[target];
    if (provider) return { kind: "provider", provider, text: rest };
  }
  // Spoken commands lead with the name and no punctuation: "grok what's new".
  const first = text.split(/\s+/, 1)[0] ?? "";
  const spoken = ALIASES[norm(first)];
  if (spoken && text.length > first.length) return { kind: "provider", provider: spoken, text: text.slice(first.length).trim() };
  return { kind: "default", text };
}
