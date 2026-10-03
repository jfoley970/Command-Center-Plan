// The thin toolbar in the top-right margin. Each tool is one item; new ones go
// to the left of the timer so the timer stays in the corner.
import Pomodoro from "./Pomodoro";

export default function TopBar() {
  return (
    <div className="topbar" role="toolbar" aria-label="Toolbar">
      <Pomodoro />
    </div>
  );
}
