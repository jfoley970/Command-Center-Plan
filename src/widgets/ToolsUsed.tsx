import type { ToolUsed } from "../api";

/** What Local AI looked at or changed to answer, under its reply. */
export function ToolsUsed({ tools }: { tools?: ToolUsed[] }) {
  if (!tools?.length) return null;
  return (
    <ul className="tools-used" aria-label="Tools used">
      {tools.map((t, i) => (
        <li key={i} className={!t.ok ? "failed" : t.waiting ? "waiting" : ""} title={t.tool}>
          {t.label}
        </li>
      ))}
    </ul>
  );
}
