import Widget from "./Widget";

/** Placeholders for milestone 2 connections so the layout already has room for them. */
export default function Integrations() {
  const rows = [
    { name: "Email digest", detail: "What needs a reply, what is FYI, what to skip", when: "Milestone 2" },
    { name: "ESXi host", detail: "VM power state, host health, datastore space", when: "Milestone 2" },
    { name: "Calendar", detail: "Today's meetings on the timeline", when: "Milestone 2" },
    { name: "Voice", detail: "Push-to-talk commands", when: "Milestone 2" },
  ];
  return (
    <Widget title="Connections">
      <ul className="list dense">
        {rows.map((r) => (
          <li key={r.name}>
            <span className="status-dot idle" aria-hidden="true" />
            <span className="grow truncate">
              {r.name}
              <span className="sub">{r.detail}</span>
            </span>
            <span className="tag">{r.when}</span>
          </li>
        ))}
      </ul>
    </Widget>
  );
}
