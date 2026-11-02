// Push-to-talk through Whisper on the server. No key: only cc-server reaches
// Whisper, so this card only reports whether it's up.
import { useEffect, useState } from "react";
import { voice, type VoiceStatus } from "../voice";
import { registerConnector } from "./registry";

function useVoiceStatus() {
  const [s, setS] = useState<VoiceStatus | null>(null);
  const check = () => voice.status().then(setS).catch((e) => setS({ connected: false, note: String(e) }));
  useEffect(() => {
    check();
  }, []);
  return { s, check };
}

function VoicePanel() {
  const { s, check } = useVoiceStatus();
  return (
    <div className="stack-tight">
      <p className="muted small">{s?.note ?? "Checking Whisper…"}</p>
      <p className="muted small">
        Click the mic in the Agents widget, speak, and click again. Whisper on the server writes it down, so the
        recording never leaves it. Say “Claude, …”, “Grok, …”, an agent's name or one of your trigger words to pick
        who answers; “add a todo …” or “remind me at 3 PM …” does that; anything else goes to the agent on screen.
        Whisper starts with Local AI on the server (<code>docker compose up -d whisper</code>); see the server
        README, “Voice (Whisper)”.
      </p>
      <div className="form-row">
        <button type="button" className="ghost" onClick={check}>Check again</button>
      </div>
    </div>
  );
}

registerConnector({
  id: "voice",
  name: "Voice (Whisper)",
  group: "AI",
  description: "Push-to-talk from the Agents widget, written down on your own server.",
  useStatus: () => {
    const { s } = useVoiceStatus();
    if (!s) return { state: "off", detail: "Checking Whisper…" };
    return s.connected ? { state: "connected", detail: s.note } : { state: "off", detail: s.note };
  },
  Panel: VoicePanel,
});
