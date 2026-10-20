// Local models through Ollama on the server. No key: the server reaches Ollama
// on its own private network, so this card only reports what's there.
import { useData } from "../data";
import { registerConnector } from "./registry";

function OllamaPanel() {
  const { providers, refresh } = useData();
  const p = providers.find((x) => x.id === "local");
  return (
    <div className="stack-tight">
      {p?.models.length ? (
        <ul className="muted small">
          {p.models.map((m) => <li key={m.id}>{m.label}</li>)}
        </ul>
      ) : (
        <p className="muted small">{p?.note ?? "Checking Ollama…"}</p>
      )}
      <p className="muted small">
        Pull a model on the server with <code>docker compose exec ollama ollama pull qwen3:8b</code>, then
        make an agent with Local AI as its provider, or type <code>local: …</code> in the ask box. Prompts and
        answers stay on the server.
      </p>
      <div className="form-row">
        <button type="button" className="ghost" onClick={() => refresh()}>Check again</button>
      </div>
    </div>
  );
}

registerConnector({
  id: "local",
  name: "Local AI (Ollama)",
  group: "AI",
  description: "Runs open models on your own server, free per use, for drafts, triage and summaries that shouldn't leave the building.",
  useStatus: () => {
    const { providers } = useData();
    const p = providers.find((x) => x.id === "local");
    if (!p) return { state: "off", detail: "Checking Ollama…" };
    if (p.connected) return { state: "connected", detail: p.note ?? "Ready" };
    return { state: p.note?.startsWith("Ollama is running") ? "attention" : "off", detail: p.note ?? "Not running" };
  },
  Panel: OllamaPanel,
});
