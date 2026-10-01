import { useState, type FormEvent } from "react";
import { api } from "../api";
import { keyStore } from "../transport";
import { useData } from "../data";
import { registerConnector } from "./registry";

function ClaudePanel() {
  const { hasKey, act } = useData();
  const [key, setKey] = useState("");
  const [saved, setSaved] = useState(false);

  async function save(e: FormEvent) {
    e.preventDefault();
    setSaved(false);
    const ok = await act(async () => {
      await api.setApiKey(key);
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
          placeholder={hasKey ? "Paste a new key to replace it" : "sk-ant-…"}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          aria-label="Claude API key"
          autoComplete="off"
        />
        <button className="primary" type="submit" disabled={!key.trim()}>Save</button>
        {hasKey && (
          <button type="button" className="ghost" onClick={() => act(() => api.setApiKey(""))}>
            Remove
          </button>
        )}
      </div>
      <p className="muted small">Stored in {keyStore}, never in a plain file.{saved && " Saved."}</p>
    </form>
  );
}

registerConnector({
  id: "claude",
  name: "Claude API",
  group: "AI",
  description: "Runs your Claude agents and writes the email digests.",
  useStatus: () => {
    const { hasKey, agents } = useData();
    const n = agents.filter((a) => a.provider === "claude").length;
    return hasKey
      ? { state: "connected", detail: `Key saved · ${n} agent${n === 1 ? "" : "s"}` }
      : { state: "off", detail: "Add an API key to run agents" };
  },
  Panel: ClaudePanel,
});
