import { useState, type FormEvent } from "react";
import { api } from "../api";
import { useData } from "../data";

export default function Settings() {
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
    <div className="page">
      <header className="page-head">
        <h1>Settings</h1>
      </header>

      <form className="card stack" onSubmit={save}>
        <h2>Claude API key</h2>
        <p className="muted">
          Stored in your operating system's keychain, never in a file. Status:{" "}
          <strong>{hasKey ? "saved" : "not set"}</strong>
        </p>
        <div className="form-row">
          <input
            className="grow"
            type="password"
            placeholder={hasKey ? "Paste a new key to replace it" : "sk-ant-…"}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            autoComplete="off"
          />
          <button className="primary" type="submit" disabled={!key.trim()}>Save</button>
          {hasKey && (
            <button type="button" className="ghost" onClick={() => act(() => api.setApiKey(""))}>
              Remove
            </button>
          )}
        </div>
        {saved && <p className="muted">Saved.</p>}
      </form>

      <section className="card stack">
        <h2>Shortcuts</h2>
        <ul className="list plain">
          <li><kbd>Ctrl/Cmd</kbd> + <kbd>K</kbd> opens quick actions inside the app.</li>
          <li><kbd>Ctrl/Cmd</kbd> + <kbd>Shift</kbd> + <kbd>Space</kbd> brings the app forward from anywhere.</li>
          <li>Closing the window keeps the app in the tray so reminders still fire. Quit from the tray icon.</li>
        </ul>
      </section>

      <section className="card soon">
        <h2>Coming next</h2>
        <p className="muted">Email account, calendar, ESXi host connection and voice settings arrive in milestone 2.</p>
      </section>
    </div>
  );
}
