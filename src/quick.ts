// Quick actions shared by the dashboard command bar and the Ctrl/Cmd+K palette.
import { api } from "./api";

export function tomorrowAt9(): Date {
  const d = new Date();
  d.setDate(d.getDate() + 1);
  d.setHours(9, 0, 0, 0);
  return d;
}

export function minutesFromNow(m: number): Date {
  return new Date(Date.now() + m * 60 * 1000);
}

export const quick = {
  todo: (title: string) => api.addTodo({ title }),
  remind: (title: string, at: Date) => api.addReminder({ title, remind_at: at.toISOString(), repeat: "none" }),
};
