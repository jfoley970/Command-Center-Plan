// Push-to-talk: record in the browser, Whisper on the server writes it down,
// and the words are handled like a Pocket command (crates/cc-core/src/voice.rs).
import { useEffect, useRef, useState } from "react";
import type { AgentRun, ProviderId } from "./api";
import { call } from "./transport";

export type VoiceStatus = { connected: boolean; note: string };

export type Heard = {
  heard: string;
  kind: "agent" | "todo" | "reminder" | "project";
  /** What happened, for anything but an agent's answer. */
  result: string;
  run: AgentRun | null;
};

export const voice = {
  status: () => call<VoiceStatus>("voice_status"),
  /** Who answers when the words don't name anyone: the agent or AI on screen. */
  command: (audio: string, mime: string, fallback: { agentId?: number; provider?: ProviderId }) =>
    call<Heard>("voice_command", { audio, mime, agentId: fallback.agentId ?? null, provider: fallback.provider ?? null }),
};

/** Longest recording; it stops on its own after this. */
export const MAX_SECONDS = 120;

/** Microphone recording. `stop()` resolves with the audio as base64 and its type. */
export function useRecorder() {
  const [recording, setRecording] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const rec = useRef<MediaRecorder | null>(null);
  const done = useRef<((r: { audio: string; mime: string } | null) => void) | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => {
    clearInterval(timer.current);
    rec.current?.stream.getTracks().forEach((t) => t.stop());
  }, []);

  const supported = typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== "undefined";

  async function start() {
    if (!supported) throw new Error("This browser can't record audio. Use the dashboard over HTTPS in Chrome, Edge or Safari.");
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true }).catch(() => {
      throw new Error("The microphone is blocked. Allow it for this site in the browser's address bar, then try again.");
    });
    const type = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg"].find((t) => MediaRecorder.isTypeSupported(t));
    const r = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    const chunks: Blob[] = [];
    r.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    r.onstop = async () => {
      stream.getTracks().forEach((t) => t.stop());
      clearInterval(timer.current);
      setRecording(false);
      const blob = new Blob(chunks, { type: r.mimeType || type || "audio/webm" });
      const finish = done.current;
      done.current = null;
      if (!finish) return;
      if (!blob.size) return finish(null);
      finish({ audio: await toBase64(blob), mime: blob.type });
    };
    rec.current = r;
    r.start();
    setSeconds(0);
    setRecording(true);
    const began = Date.now();
    timer.current = window.setInterval(() => {
      const s = Math.floor((Date.now() - began) / 1000);
      setSeconds(s);
      if (s >= MAX_SECONDS) r.state === "recording" && r.stop();
    }, 250);
  }

  function stop(): Promise<{ audio: string; mime: string } | null> {
    return new Promise((resolve) => {
      const r = rec.current;
      if (!r || r.state !== "recording") return resolve(null);
      done.current = resolve;
      r.stop();
    });
  }

  /** Stops without sending anything. */
  function cancel() {
    done.current = null;
    if (rec.current?.state === "recording") rec.current.stop();
  }

  return { supported, recording, seconds, start, stop, cancel };
}

function toBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}
