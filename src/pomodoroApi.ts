// Typed wrappers around the pomodoro commands in crates/cc-core/src/commands.rs.
import { call } from "./transport";

export type Phase = "work" | "break";

export type PomodoroSnapshot = {
  phase: Phase;
  minutes: number;
  state: "idle" | "running" | "paused";
  /** Time left when the backend answered; count down from when it arrived. */
  remaining_ms: number;
  /** The session that just ran out, until someone answers the popup. */
  finished: { phase: Phase; at: string } | null;
};

export const pomodoro = {
  get: () => call<PomodoroSnapshot>("pomodoro_get"),
  start: (phase: Phase, minutes?: number) => call<PomodoroSnapshot>("pomodoro_start", { phase, minutes: minutes ?? null }),
  pause: () => call<PomodoroSnapshot>("pomodoro_pause"),
  resume: () => call<PomodoroSnapshot>("pomodoro_resume"),
  reset: () => call<PomodoroSnapshot>("pomodoro_reset"),
  dismiss: () => call<PomodoroSnapshot>("pomodoro_dismiss"),
};
