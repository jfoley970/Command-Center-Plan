import { useData } from "../data";
import Widget from "./Widget";

const DAY = 86400_000;

function startOfDay(d: Date) {
  const s = new Date(d);
  s.setHours(0, 0, 0, 0);
  return s;
}

/** Todos completed per day over the last 7 days, with the week-over-week total. */
export default function Trend() {
  const { todos } = useData();
  const today = startOfDay(new Date());
  const doneTimes = todos.filter((t) => t.done && t.done_at).map((t) => new Date(t.done_at!).getTime());

  const days = Array.from({ length: 7 }, (_, i) => {
    const day = new Date(today.getTime() - (6 - i) * DAY);
    const from = day.getTime();
    const count = doneTimes.filter((t) => t >= from && t < from + DAY).length;
    return { day, count };
  });
  const thisWeek = days.reduce((s, d) => s + d.count, 0);
  const prevFrom = today.getTime() - 13 * DAY;
  const lastWeek = doneTimes.filter((t) => t >= prevFrom && t < prevFrom + 7 * DAY).length;
  const max = Math.max(4, ...days.map((d) => d.count));
  const delta = thisWeek - lastWeek;

  return (
    <Widget title="Todos completed, last 7 days">
      <div className="trend">
        <div className="hero">
          <span className="hero-value">{thisWeek}</span>
          <span className="muted small">
            {delta === 0 ? "same as" : delta > 0 ? `${delta} more than` : `${-delta} fewer than`} the week before ({lastWeek})
          </span>
        </div>
        <div className="bars" role="img" aria-label={days.map((d) => `${d.day.toLocaleDateString([], { weekday: "long" })}: ${d.count}`).join(", ")}>
          <div className="bars-grid" aria-hidden="true">
            <span style={{ bottom: "100%" }}>{max}</span>
            <span style={{ bottom: "50%" }}>{Math.round(max / 2)}</span>
          </div>
          {days.map((d, i) => (
            <div className="bar-col" key={i}>
              <div className="bar-hit">
                <div className="bar" style={{ height: `${(d.count / max) * 100}%` }} />
                <span className="bar-tip">
                  <strong>{d.count}</strong> completed
                  <span>{d.day.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })}</span>
                </span>
              </div>
              <span className={i === 6 ? "bar-label today" : "bar-label"}>
                {i === 6 ? "Today" : d.day.toLocaleDateString([], { weekday: "short" })}
              </span>
            </div>
          ))}
        </div>
      </div>
    </Widget>
  );
}
