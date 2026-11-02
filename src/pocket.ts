// Typed wrappers around the Pocket recorder commands in crates/cc-core/src/commands.rs.
import { call } from "./transport";

export type PocketAuto = "tasks" | "all" | "none";
export type PocketKind = "todo" | "reminder" | "project" | "agent";

export type PocketCommand = {
  id: number;
  recording_id: string;
  recorded_at: string | null;
  /** The words after the wake phrase, as Pocket transcribed them. */
  heard: string;
  kind: PocketKind;
  title: string;
  due_at: string | null;
  target: string;
  target_name: string;
  status: "pending" | "done" | "dismissed" | "error";
  result: string;
  created_at: string;
  /** "local" when the local model worked out what the command meant. */
  via: "rules" | "local";
  /** Where to see the outcome in the app, such as "#agents/2" for an agent's chat. */
  link: string;
};

/** A word that, opening a recording, sends the rest somewhere you chose. */
export type PocketTrigger = {
  word: string;
  /** "provider:claude", "agent:3", or "todo" | "reminder" | "project". */
  target: string;
  /** Filled in by the server when saved. */
  target_name?: string;
};

export type PocketSnapshot = {
  has_key: boolean;
  has_webhook_secret: boolean;
  wake: string;
  auto: PocketAuto;
  /** The local model reads commands the simple rules can't place. */
  ai: boolean;
  /** Empty means the smallest model pulled into Ollama. */
  ai_model: string;
  since: string | null;
  checked_at: string | null;
  error: string | null;
  commands: PocketCommand[];
  triggers: PocketTrigger[];
};

export type PocketEdit = { kind?: PocketKind; title?: string; due_at?: string | null };

export const pocket = {
  status: () => call<PocketSnapshot>("pocket_status"),
  refresh: () => call<PocketSnapshot>("pocket_refresh"),
  setKey: (key: string) => call<PocketSnapshot>("pocket_set_key", { key }),
  setWebhookSecret: (key: string) => call<PocketSnapshot>("pocket_set_webhook_secret", { key }),
  setOptions: (o: { wake: string; auto: PocketAuto; ai: boolean; aiModel: string }) => call<PocketSnapshot>("pocket_set_options", o),
  setTriggers: (triggers: PocketTrigger[]) => call<PocketSnapshot>("pocket_set_triggers", { triggers }),
  run: (id: number, edit?: PocketEdit) => call<PocketCommand>("pocket_run", { id, edit }),
  dismiss: (id: number) => call<void>("pocket_dismiss", { id }),
};

export const AUTO_LABEL: Record<PocketAuto, string> = {
  tasks: "Todos, reminders and questions to AIs run at once; Cursor jobs wait for a tap",
  all: "Everything runs at once, Cursor jobs included",
  none: "Everything waits for a tap",
};
