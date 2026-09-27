import type { ReactNode } from "react";

/** Card chrome for a dashboard widget. The header is the drag handle. */
export default function Widget(props: { title: string; actions?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`widget ${props.className ?? ""}`}>
      <header className="widget-head">
        <span className="drag-handle" title="Drag to move">
          <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
            {[2, 6, 10].flatMap((y) => [3, 9].map((x) => <circle key={`${x}-${y}`} cx={x} cy={y} r="1.1" fill="currentColor" />))}
          </svg>
          <h2>{props.title}</h2>
        </span>
        <div className="widget-actions">{props.actions}</div>
      </header>
      <div className="widget-body">{props.children}</div>
    </section>
  );
}
