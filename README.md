# Command Center

A desktop app for Mac and Windows that brings your day and your AI agents into one place: a dashboard, quick actions, todos, reminders and an agent hub. Built with Tauri 2, React and TypeScript, with a Rust backend.

## What's in milestone 1

- **Interactive dashboard (dark theme):** a capture bar, clickable stat tiles, a live timeline of today's reminders and due todos, and widgets for todos, reminders, an agent console, a 7-day completion chart and upcoming connections. Drag a widget by its title to move it, drag its corner to resize it, and the layout is remembered. Most work happens in place, without leaving the dashboard.
- **Quick actions:** press `Ctrl/Cmd + K` in the app, or `Ctrl/Cmd + Shift + Space` from anywhere. Type any text to add it as a todo, set a reminder for 30 minutes or tomorrow at 9 AM, or run an agent with it.
- **Todos:** priority, optional due date, completion history.
- **Reminders:** one-off, daily or weekly, shown as desktop notifications. Closing the window keeps the app in the system tray so reminders still fire.
- **Agent hub:** define agents (name, instructions, model), run them, and keep a history of every run with token counts. Each run automatically includes your open todos and upcoming reminders. A "Daily Planner" agent is included.
- **Settings:** your Claude API key is stored in the OS keychain (macOS Keychain or Windows Credential Manager), never in a file.

Data is stored locally in SQLite in the app's data folder.

## Getting installers

Every push to `main` builds a `.dmg` for macOS and `.msi` / `.exe` installers for Windows in GitHub Actions. Download them from the run's **Artifacts** section. The builds are not code-signed yet, so macOS will ask you to right-click and choose Open the first time, and Windows SmartScreen may show "More info, Run anyway".

## Developing

Prerequisites: Node 22+, Rust (stable), and the [Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev      # run the app with hot reload
npm run build          # typecheck and build the frontend
cargo test --manifest-path src-tauri/Cargo.toml   # backend tests
npm run tauri build    # build an installer for this machine
```

For development you can set `ANTHROPIC_API_KEY` in your environment instead of saving a key in Settings.

## Layout

```
src/                 React frontend
  App.tsx            shell, navigation, quick-action palette
  data.tsx           shared data store, refreshes on backend events
  api.ts             typed wrappers for backend commands
  pages/             Dashboard, Todos, Reminders, Agents, Settings
  widgets/           dashboard widgets (timeline, todos, reminders, agent console, trend…)
  quick.ts           quick actions shared by the capture bar and palette
src-tauri/src/
  lib.rs             commands, tray, global shortcut, reminder loop
  db.rs              SQLite schema and queries (with tests)
  claude.rs          Claude Messages API client
  secrets.rs         OS keychain access
```

## Roadmap

- **Milestone 2:** email digest with draft replies (Microsoft 365 or Gmail), calendar on the dashboard, push-to-talk voice commands, read-only ESXi host status (VMs, host health, datastores), and agent tools via MCP.
- **Milestone 3:** reporting, ESXi actions (power, snapshots, console) with confirmations, opt-in wake-word listening.
