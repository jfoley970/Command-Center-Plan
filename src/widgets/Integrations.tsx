import { useData } from "../data";
import Widget from "./Widget";

/** Live email status, plus placeholders for later connections so the layout already has room for them. */
export default function Integrations({ onOpenInbox }: { onOpenInbox: () => void }) {
  const { mailAccounts } = useData();
  const pending = mailAccounts.reduce((n, a) => n + a.pending, 0);
  const unread = mailAccounts.reduce((n, a) => n + a.unread, 0);
  const failing = mailAccounts.some((a) => a.last_error);

  const rows = [
    { name: "ESXi host", detail: "VM power state, host health, datastore space", when: "Later" },
    { name: "Calendar", detail: "Today's meetings on the timeline", when: "Later" },
    { name: "Voice", detail: "Push-to-talk commands", when: "Later" },
  ];
  return (
    <Widget title="Connections">
      <ul className="list dense">
        <li className="clickable" onClick={onOpenInbox}>
          <span className={`status-dot ${mailAccounts.length === 0 ? "idle" : failing ? "error" : "ok"}`} aria-hidden="true" />
          <span className="grow truncate">
            Email
            <span className="sub">
              {mailAccounts.length === 0
                ? "Connect an inbox for summaries and suggested tasks"
                : failing
                  ? "An inbox needs attention"
                  : `${mailAccounts.length} inbox${mailAccounts.length === 1 ? "" : "es"} · ${unread} unread`}
            </span>
          </span>
          {pending > 0 ? <span className="tag accent">{pending} to review</span> : <span className="tag">{mailAccounts.length ? "Open" : "Connect"}</span>}
        </li>
        {rows.map((r) => (
          <li key={r.name}>
            <span className="status-dot idle" aria-hidden="true" />
            <span className="grow truncate">
              {r.name}
              <span className="sub">{r.detail}</span>
            </span>
            <span className="tag">{r.when}</span>
          </li>
        ))}
      </ul>
    </Widget>
  );
}
