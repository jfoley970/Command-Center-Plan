// One way for the UI to reach the backend, wherever it runs. In the desktop app
// the backend is in process and calls go through Tauri IPC. In a browser the
// page was served by cc-server, so calls go to /api/call/<command> and events
// arrive over the /api/events WebSocket. Both carry the same commands and events.
import { invoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";

export const isDesktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Where keys pasted into the app end up, for help text. */
export const keyStore = isDesktop ? "your operating system's keychain" : "the server's encrypted key store";

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isDesktop) return invoke<T>("call", { command, args: args ?? null });
  const resp = await fetch(`/api/call/${command}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify(args ?? {}),
  });
  if (resp.status === 401) throw "Your sign-in expired. Reload the page to sign in again.";
  const body = await resp.json().catch(() => null);
  if (!resp.ok) throw body?.error ?? `The server returned ${resp.status}.`;
  return body as T;
}

type Handler<T> = (event: { payload: T }) => void;
type Unlisten = () => void;

// Snapshot-style events are also re-run when the backend says the client may
// have missed some ("resync"), for example after the connection dropped.
const refreshes = (name: string) => name.endsWith("-changed") || name === "reminders-fired";

const handlers = new Map<string, Set<Handler<unknown>>>();
let socket: WebSocket | null = null;
let retry = 1000;

function dispatch(name: string, payload: unknown) {
  if (name === "notify") showNotification(payload as { title: string; body: string });
  if (name === "resync") {
    for (const [event, set] of handlers) if (refreshes(event)) set.forEach((h) => h({ payload: null }));
    return;
  }
  handlers.get(name)?.forEach((h) => h({ payload }));
}

function connect() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${proto}//${location.host}/api/events`);
  socket = ws;
  let opened = false;
  ws.onopen = () => {
    // Anything that changed while we were away is picked up now.
    if (opened || retry > 1000) dispatch("resync", null);
    opened = true;
    retry = 1000;
  };
  ws.onmessage = (m) => {
    try {
      const { name, payload } = JSON.parse(m.data);
      dispatch(name, payload);
    } catch {
      // Ignore anything that isn't an event.
    }
  };
  ws.onclose = () => {
    socket = null;
    setTimeout(connect, retry);
    retry = Math.min(retry * 2, 30_000);
  };
}

/** Same shape as Tauri's `listen`, so callers don't care where the backend is. */
export async function listen<T>(event: string, handler: Handler<T>): Promise<Unlisten> {
  if (isDesktop) {
    const offs = [await tauriListen<T>(event, handler)];
    if (refreshes(event)) offs.push(await tauriListen("resync", () => handler({ payload: null as T })));
    return () => offs.forEach((off) => off());
  }
  if (!socket) connect();
  const set = handlers.get(event) ?? new Set();
  set.add(handler as Handler<unknown>);
  handlers.set(event, set);
  return () => set.delete(handler as Handler<unknown>);
}

export function openUrl(url: string): Promise<void> {
  if (isDesktop) return tauriOpenUrl(url);
  window.open(url, "_blank", "noopener,noreferrer");
  return Promise.resolve();
}

// ---------- Browser notifications ----------
// The desktop app shows OS notifications itself; in a browser we ask once, on
// the first click, because browsers ignore permission requests without one.

function showNotification({ title, body }: { title: string; body: string }) {
  if (isDesktop || typeof Notification === "undefined" || Notification.permission !== "granted") return;
  new Notification(title, { body });
}

if (!isDesktop && typeof window !== "undefined" && typeof Notification !== "undefined" && Notification.permission === "default") {
  window.addEventListener("click", () => void Notification.requestPermission(), { once: true });
}
