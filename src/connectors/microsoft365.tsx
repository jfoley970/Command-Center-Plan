import { useData } from "../data";
import { registerConnector } from "./registry";
import type { Page } from "../App";

function MicrosoftPanel({ go }: { go: (p: Page) => void }) {
  const { mailAccounts } = useData();
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
