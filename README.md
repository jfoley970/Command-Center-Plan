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

## Server and clients

The backend is a shared Rust crate (`crates/cc-core`) that runs in two places:

- **In the desktop app** (local mode), as it always has.
- **On a server** (`crates/cc-server`), which also serves the same UI to any browser. This is the target setup: one Linux box holds the data and keys and does the polling, and any browser, at home or away, reaches it at a public name behind your own sign-in with MFA. See [deploy/README.md](deploy/README.md).

The desktop app can also run on the server (Settings > Command Center server). Then the desktop app and the web app are mirror images: one set of data, connectors and dashboard layout, with the desktop app adding the tray, the global shortcut and OS notifications. The same card copies connectors set up on the desktop to the server. Every connector lives in `cc-core`, so a connector added for one is there for both.

## Getting installers

Every push to `main` builds a `.dmg` for macOS and `.msi` / `.exe` installers for Windows in GitHub Actions. Download them from the run's **Artifacts** section. The builds are not code-signed yet, so macOS will ask you to right-click and choose Open the first time, and Windows SmartScreen may show "More info, Run anyway".

## Developing

Prerequisites: Node 22+, Rust (stable), and the [Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev      # run the app with hot reload
npm run build          # typecheck and build the frontend
cargo test --workspace # backend tests
npm run tauri build    # build an installer for this machine

# The server, with the UI in a browser:
cargo run -p cc-server # http://127.0.0.1:8484, serves ./dist (run npm run build first)
npm run dev            # or: Vite on :1420, proxying /api to the server
```

For development you can set `ANTHROPIC_API_KEY` in your environment instead of saving a key in Settings.

## Layout

```
src/                 React frontend
  App.tsx            shell, navigation, quick-action palette
  data.tsx           shared data store, refreshes on backend events
  api.ts             typed wrappers for backend commands
  transport.ts       Tauri IPC in the desktop app, HTTP + WebSocket in a browser
  pages/             Dashboard, Todos, Reminders, Agents, Settings
  widgets/           dashboard widgets (timeline, todos, reminders, agent console, trend…)
  quick.ts           quick actions shared by the capture bar and palette
crates/cc-core/src/  the backend, shared by the desktop app and the server
  commands.rs        every command the UI can call, by name
  lib.rs             Core (database, secrets, events) and background loops
  db.rs              SQLite schema and queries (with tests)
  claude.rs          Claude Messages API client
  mail.rs            Microsoft 365 sign-in, Graph sync, digests
  atera.rs, unifi.rs connectors
  transfer.rs        moving connectors from the desktop app to the server
  secrets.rs         OS keychain (desktop) or encrypted file (server)
crates/cc-server/    HTTP + WebSocket API, sign-in checks, serves the web UI
src-tauri/src/lib.rs desktop shell: tray, global shortcut, notifications
src-tauri/src/remote.rs server mode: the desktop app as a client of cc-server
deploy/              Docker Compose, Caddy and setup notes for the server
```

## Roadmap

- **Milestone 2:** email digest with draft replies (Microsoft 365 or Gmail), calendar on the dashboard, push-to-talk voice commands, read-only ESXi host status (VMs, host health, datastores), and agent tools via MCP.
- **Milestone 3:** reporting, ESXi actions (power, snapshots, console) with confirmations, opt-in wake-word listening.
