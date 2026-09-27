import { useEffect, useState } from "react";
import { api } from "../api";
import { useData } from "../data";
import { isToday } from "../time";
import Widget from "./Widget";

type Item = {
  key: string;
  kind: "reminder" | "todo";
  id: number;
  title: string;
  at: Date;
  lane: number;
};

const DEFAULT_START = 6;
const DEFAULT_END = 22;
const LANES = 3;

function hourOf(d: Date) {
  return d.getHours() + d.getMinutes() / 60;
}

function timeLabel(d: Date) {
  return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** Today on one axis: reminders (circles) and todos due (diamonds), with a live "now" line. */
export default function Timeline() {
  const { todos, reminders, act } = useData();
  const [now, setNow] = useState(() => new Date());
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(t);
  }, []);

  const raw: Omit<Item, "lane">[] = [
    ...reminders
      .filter((r) => isToday(r.remind_at))
      .map((r) => ({ key: `r${r.id}`, kind: "reminder" as const, id: r.id, title: r.title, at: new Date(r.remind_at) })),
    ...todos
      .filter((t) => !t.done && isToday(t.due_at))
      .map((t) => ({ key: `t${t.id}`, kind: "todo" as const, id: t.id, title: t.title, at: new Date(t.due_at!) })),
  ].sort((a, b) => a.at.getTime() - b.at.getTime());

  // The window always covers the working day, now, and every item.
  const start = Math.min(DEFAULT_START, Math.floor(hourOf(now)), ...raw.map((i) => Math.floor(hourOf(i.at))));
  const end = Math.min(24, Math.max(DEFAULT_END, Math.ceil(hourOf(now) + 0.01), ...raw.map((i) => Math.ceil(hourOf(i.at) + 0.01))));
  const pct = (d: Date) => ((hourOf(d) - start) / (end - start)) * 100;

  // Stagger markers that sit close together into separate lanes.
  const lastInLane: number[] = Array(LANES).fill(-Infinity);
  const items: Item[] = raw.map((i) => {
    const p = pct(i.at);
    let lane = lastInLane.findIndex((last) => p - last > 4);
    if (lane === -1) lane = lastInLane.indexOf(Math.min(...lastInLane));
    lastInLane[lane] = p;
    return { ...i, lane };
  });

  const hours = Array.from({ length: end - start + 1 }, (_, i) => start + i);
  const nowPct = pct(now);
  const selected = items.find((i) => i.key === open);

  return (
    <Widget
      title="Today"
      actions={
        <div className="legend" aria-label="Legend">
          <span><i className="mk reminder" /> Reminder</span>
          <span><i className="mk todo" /> Todo due</span>
        </div>
      }
    >
      <div className="timeline" onMouseLeave={() => setOpen(null)}>
        <div className="tl-track">
          {hours.map((h) => (
            <div key={h} className="tl-hour" style={{ left: `${((h - start) / (end - start)) * 100}%` }}>
              <span>{h % 12 === 0 ? 12 : h % 12}{h < 12 || h === 24 ? "a" : "p"}</span>
            </div>
          ))}
          {nowPct >= 0 && nowPct <= 100 && (
            <div className="tl-now" style={{ left: `${nowPct}%` }}>
              <span>Now {timeLabel(now)}</span>
            </div>
          )}
          {items.map((i) => (
            <button
              key={i.key}
              className={`tl-item ${i.kind} ${i.at < now ? "past" : ""} ${open === i.key ? "active" : ""}`}
              style={{ left: `${pct(i.at)}%`, top: `calc(50% - 22px + ${i.lane * 22}px)` }}
              onClick={() => setOpen(open === i.key ? null : i.key)}
              aria-label={`${i.kind === "todo" ? "Todo due" : "Reminder"}: ${i.title} at ${timeLabel(i.at)}`}
            >
              <span className="tl-tip">
                <strong>{i.title}</strong>
                <span>{i.kind === "todo" ? "Todo due" : "Reminder"} · {timeLabel(i.at)}</span>
              </span>
            </button>
          ))}
        </div>
        {items.length === 0 && <p className="empty tl-empty">Nothing scheduled for today. Reminders and todos due today show up here.</p>}
        {selected && (
          <div
            className="tl-pop"
            style={{
              left: `clamp(0px, calc(${pct(selected.at)}% - 110px), calc(100% - 220px))`,
              top: `calc(50% + ${selected.lane * 22 + 4}px)`,
            }}
          >
            <strong className="truncate">{selected.title}</strong>
            <span className="muted small">{selected.kind === "todo" ? "Todo due" : "Reminder"} at {timeLabel(selected.at)}</span>
            <div className="row">
              {selected.kind === "todo" ? (
                <button className="primary" onClick={() => { setOpen(null); act(() => api.setTodoDone(selected.id, true)); }}>
                  Mark done
                </button>
              ) : (
                <>
                  <button onClick={() => { setOpen(null); act(() => api.snoozeReminder(selected.id, 15)); }}>In 15 min</button>
                  <button onClick={() => { setOpen(null); act(() => api.deleteReminder(selected.id)); }}>Remove</button>
                </>
              )}
            </div>
          </div>
        )}
      </div>
    </Widget>
  );
}
