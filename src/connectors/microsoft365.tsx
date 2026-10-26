import { useEffect, useState } from "react";
import { api } from "../api";
import { useData } from "../data";
import { registerConnector } from "./registry";
import type { Page } from "../App";

/** The server checks every inbox this often; it never goes below 10 seconds. */
const MIN_SYNC_SECONDS = 10;

function MicrosoftPanel({ go }: { go: (p: Page) => void }) {
  const { mailAccounts, act } = useData();
  const [seconds, setSeconds] = useState("");

  useEffect(() => {
    api.getMailSetup().then((s) => setSeconds(String(s.sync_seconds))).catch(() => {});
  }, []);

  async function saveSeconds() {
    const n = Math.max(MIN_SYNC_SECONDS, Math.round(Number(seconds)) || MIN_SYNC_SECONDS);
    const saved = await act(() => api.setMailSyncSeconds(n));
    if (saved !== undefined) setSeconds(String(saved));
  }

  return (
    <div className="stack-tight">
      {mailAccounts.length > 0 && (
        <ul className="list dense">
          {mailAccounts.map((a) => (
            <li key={a.id}>
              <span className={`status-dot ${a.last_error ? "error" : "ok"}`} aria-hidden="true" />
              <span className="grow truncate">
                {a.email}
                <span className="sub">{a.last_error ?? `${a.unread} unread`}</span>
              </span>
            </li>
          ))}
        </ul>
      )}
      <label className="row">
        <span className="grow">Check for new mail every</span>
        <input
          type="number"
          min={MIN_SYNC_SECONDS}
          step={1}
          value={seconds}
          onChange={(e) => setSeconds(e.target.value)}
          onBlur={saveSeconds}
          onKeyDown={(e) => e.key === "Enter" && saveSeconds()}
          className="seconds-input"
          aria-label="Seconds between mail checks"
        />
        <span>seconds</span>
      </label>
      <div className="row">
        <button onClick={() => go("inbox")}>{mailAccounts.length ? "Manage inboxes" : "Connect an inbox"}</button>
      </div>
    </div>
  );
}

registerConnector({
  id: "microsoft365",
  name: "Microsoft 365",
  group: "Email",
  description: "Reads your inboxes for summaries, suggested tasks and flagged-email todos.",
  useStatus: () => {
    const { mailAccounts } = useData();
    if (mailAccounts.length === 0) return { state: "off", detail: "No inbox connected" };
    const failing = mailAccounts.find((a) => a.last_error);
    if (failing) return { state: "attention", detail: `${failing.email}: ${failing.last_error}` };
    return { state: "connected", detail: `${mailAccounts.length} inbox${mailAccounts.length === 1 ? "" : "es"}` };
  },
  Panel: MicrosoftPanel,
});
