import { useEffect, useState } from "react";
import { api, type MailAction, type ToolApproval } from "../api";
import { useData } from "../data";
import { listen } from "../transport";

/**
 * What Local AI asked to do that waits for you: deletes, asking the paid AIs,
 * Cursor work, and (from the inbox's own queue) mail it wants to send or delete.
 * Nothing here runs until you press Approve.
 */
export function Approvals() {
  const { act } = useData();
  const [tools, setTools] = useState<ToolApproval[]>([]);
  const [mail, setMail] = useState<MailAction[]>([]);

  useEffect(() => {
    const load = () => {
      api.listToolApprovals().then(setTools).catch(() => {});
      api.listMailActions().then(setMail).catch(() => {});
    };
    load();
    const uns = [listen("approvals-changed", load), listen("mail-actions-changed", load)];
    return () => uns.forEach((u) => void u.then((f) => f()));
  }, []);

  // Asking Grok runs without a tap, so it never waits here.
  const pendingTools = tools.filter((a) => a.status === "pending" && !a.args.includes('"auto":true'));
  const pendingMail = mail.filter((a) => a.status === "pending" && a.requested_by === "Local AI");
  if (pendingTools.length === 0 && pendingMail.length === 0) return null;

  return (
    <section className="approvals" aria-label="Waiting for your approval">
      <h3>Local AI is waiting for your OK</h3>
      <ul>
        {pendingTools.map((a) => (
          <li key={`t${a.id}`}>
            <span className="grow">{a.summary}</span>
            <button className="primary small" onClick={() => act(() => api.approveTool(a.id))}>Approve</button>
            <button className="ghost small" onClick={() => act(() => api.rejectTool(a.id))}>Reject</button>
          </li>
        ))}
        {pendingMail.map((a) => (
          <li key={`m${a.id}`}>
            <span className="grow">{a.summary}</span>
            <button className="primary small" onClick={() => act(() => api.approveMailAction(a.id))}>Approve</button>
            <button className="ghost small" onClick={() => act(() => api.rejectMailAction(a.id))}>Reject</button>
          </li>
        ))}
      </ul>
    </section>
  );
}
