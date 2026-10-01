// ChatGPT, Grok, Cursor and Copilot. Each key is pasted here once and lives in
// the secret store; agents on that provider can run as soon as it's saved.
import { useState, type FormEvent } from "react";
import { api, type ProviderId } from "../api";
import { keyStore } from "../transport";
import { useData } from "../data";
import { registerConnector, type ConnectorStatus } from "./registry";

function KeyPanel({ provider, placeholder, help }: { provider: ProviderId; placeholder: string; help: string }) {
  const { providers, act } = useData();
  const connected = providers.find((p) => p.id === provider)?.connected ?? false;
  const [key, setKey] = useState("");
  const [saved, setSaved] = useState(false);

  async function save(e: FormEvent) {
    e.preventDefault();
    setSaved(false);
    const ok = await act(async () => {
      await api.setProviderKey(provider, key);
      return true;
    });
    if (ok) {
      setKey("");
      setSaved(true);
    }
  }

  return (
    <form className="stack-tight" onSubmit={save}>
      <div className="form-row">
        <input
          className="grow"
          type="password"
          placeholder={connected ? "Paste a new key to replace it" : placeholder}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          aria-label={`${provider} API key`}
          autoComplete="off"
        />
        <button className="primary" type="submit" disabled={!key.trim()}>Save</button>
        {connected && (
          <button type="button" className="ghost" onClick={() => act(() => api.setProviderKey(provider, ""))}>
            Remove
          </button>
        )}
      </div>
      <p className="muted small">{help} Stored in {keyStore}, never in a plain file.{saved && " Saved."}</p>
    </form>
  );
}

function useProviderStatus(id: ProviderId, off: string): ConnectorStatus {
  const { providers, agents } = useData();
  const p = providers.find((x) => x.id === id);
  if (!p?.connected) return { state: "off", detail: off };
  const n = agents.filter((a) => a.provider === id).length;
  return { state: "connected", detail: `Key saved · ${n} agent${n === 1 ? "" : "s"}` };
}

registerConnector({
  id: "chatgpt",
  name: "ChatGPT (OpenAI API)",
  group: "AI",
  description: "Ask ChatGPT from the dashboard, with web search and code running. Billed per use by OpenAI, separately from a ChatGPT plan.",
  useStatus: () => useProviderStatus("chatgpt", "Add an OpenAI API key"),
  Panel: () => (
    <KeyPanel
      provider="chatgpt"
      placeholder="sk-…"
      help="Create a key at platform.openai.com > API keys, and set a monthly spend limit under Billing."
    />
  ),
});

registerConnector({
  id: "grok",
  name: "Grok (xAI API)",
  group: "AI",
  description: "Ask Grok from the dashboard. A SuperGrok or X Premium subscription doesn't include API access; the API is billed per use.",
  useStatus: () => useProviderStatus("grok", "Add an xAI API key"),
  Panel: () => (
    <KeyPanel provider="grok" placeholder="xai-…" help="Create a key at console.x.ai, and set a spending limit there." />
  ),
});

registerConnector({
  id: "cursor",
  name: "Cursor background agents",
  group: "AI",
  description: "Hands coding tasks to Cursor agents that work on a GitHub repository in the cloud and open a pull request.",
  useStatus: () => useProviderStatus("cursor", "Add a Cursor API key"),
  Panel: () => (
    <KeyPanel provider="cursor" placeholder="key_…" help="Create a key in Cursor's dashboard under Integrations > API keys." />
  ),
});

registerConnector({
  id: "copilot",
  name: "Microsoft 365 Copilot",
  group: "AI",
  description: "Can only be driven through Microsoft Graph with a Microsoft 365 Copilot license, on your non-clinic account.",
  useStatus: () => ({ state: "planned", detail: "Needs a Copilot license on the non-clinic account" }),
});
