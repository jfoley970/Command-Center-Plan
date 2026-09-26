// Date helpers. Storage is RFC 3339 UTC; the UI works in local time.

export function toLocalInput(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Converts a datetime-local input value to an RFC 3339 UTC string. */
export function fromLocalInput(value: string): string {
  return new Date(value).toISOString();
}

export function formatWhen(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const tomorrow = new Date(now);
  tomorrow.setDate(now.getDate() + 1);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (sameDay) return `Today ${time}`;
  if (d.toDateString() === tomorrow.toDateString()) return `Tomorrow ${time}`;
  return `${d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })} ${time}`;
}

export function isOverdue(iso: string | null): boolean {
  return !!iso && new Date(iso).getTime() < Date.now();
}

export function isToday(iso: string | null): boolean {
  return !!iso && new Date(iso).toDateString() === new Date().toDateString();
}
