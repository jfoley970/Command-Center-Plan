import { useEffect, useState, type FormEvent } from "react";
import { api, errorText, type AgentRun, type CursorMessage } from "../api";
import { useData } from "../data";

const REFRESH_MS = 15_000;

/**
 * Controls for a run handed to a Cursor background agent: its step-by-step
 * conversation, a box to send a follow-up, and a stop button while it works.
 */
export default function CursorRun({ run }: { run: AgentRun }) {
  const { act } = useData();
  const [open, setOpen] = useState(false);
  const [messages, setMessages] = useState<CursorMessage[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reply, setReply] = useState("");
  const running = run.status === "running";

  // Load the conversation when opened, and keep it fresh while Cursor works.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    const load = () =>
      api
        .cursorConversation(run.id)
        .then((m) => alive && (setMessages(m), setLoadError(null)))
        .catch((e) => alive && setLoadError(errorText(e)));
    load();
    const timer = running ? setInterval(load, REFRESH_MS) : undefined;
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [open, run.id, running]);

  async function send(e: FormEvent) {
    e.preventDefault();
    const text = reply.trim();
    if (!text) return;
    setReply("");
    await act(() => api.cursorFollowup(run.id, text));
  }

  function stop() {
    if (confirm("Stop this Cursor agent? Work it has already pushed stays on its branch.")) act(() => api.cursorStop(run.id));
  }

  return (
    <div className="cursor-run stack-tight">
      <div className="row">
        <button className="ghost" onClick={() => setOpen(!open)} aria-expanded={open}>
          {open ? "Hide steps" : "Show steps"}
        </button>
        {running && (
          <button className="ghost danger" onClick={stop}>Stop</button>
        )}
      </div>

      {open && (
        <ol className="cursor-steps" aria-label="Cursor conversation">
          {loadError && <li className="muted small">{loadError}</li>}
          {!loadError && !messages && <li className="muted small">Loading…</li>}
          {messages?.length === 0 && <li className="muted small">No messages yet.</li>}
          {messages?.map((m, i) => (
            <li key={i} className={m.from === "you" ? "step you" : "step"}>
              <span className="step-from">{m.from === "you" ? "You" : "Cursor"}</span>
              <span className="step-text">{m.text}</span>
            </li>
          ))}
        </ol>
      )}

      <form className="console-input" onSubmit={send}>
        <input
          value={reply}
          onChange={(e) => setReply(e.target.value)}
          placeholder={running ? "Add an instruction while it works…" : "Ask for a change on the same branch…"}
          aria-label="Follow-up for Cursor"
        />
        <button type="submit" disabled={!reply.trim()}>Reply</button>
      </form>
    </div>
  );
}
