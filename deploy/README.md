# Running Command Center on a server

The target setup (see the architecture plan in the project) is one Linux box that
runs the backend, reached from any of your machines over your own WireGuard, with
sign-in handled by a local auth service:

```
laptop ── WireGuard (UDP, public IP block) ──▶ server: Caddy (TLS + sign-in) ──▶ cc-server ──▶ M365, Atera, UniFi, ESXi…
                                                           │
                                                   Authentik (local auth)
```

Only the WireGuard port faces the internet. Caddy listens on the WireGuard
address, so the app, the API and the auth service are reachable only through the
tunnel.

## What runs where

| On the server (`cc-server`) | On each machine |
|---|---|
| The database, every API key and Microsoft token (encrypted) | A browser tab, or the desktop app |
| Polling Microsoft 365, Atera and UniFi; reminders; agent runs | Notifications, and in the desktop app the tray and global shortcut |

The same React UI talks to either backend: `src/transport.ts` uses Tauri IPC in
the desktop app and `/api/call/<command>` + the `/api/events` WebSocket in a
browser. The commands themselves live once, in `crates/cc-core/src/commands.rs`.

## Try it on a Linux VM

Ubuntu Server 24.04 with 4 vCPU, 8 GB RAM and 60 GB of disk is enough until the
GPU box exists. Install Docker (`curl -fsSL https://get.docker.com | sh`).

1. **WireGuard.** Install `wireguard-tools`, create `wg0` on the server with an
   address like `10.66.0.1/24`, and add a peer per machine. Forward the chosen UDP
   port from an address on your public IP block to the server (or run WireGuard on
   the edge router and route `10.66.0.0/24` to the server).
2. **Name.** Point the app's name at the WireGuard address (`10.66.0.1`), either
   as a DNS record only your tunnel resolves, or in each peer's `DNS =` resolver.
   The hostname can be chosen later; nothing in the code depends on it.
3. **Auth service.** Run Authentik (its own compose file, see goauthentik.io) and
   create a Proxy Provider in *forward auth (single application)* mode for the
   app's URL, plus an Application and an outpost. Turn on MFA or passkeys for
   your user.
4. **Command Center.**
   ```bash
   cd deploy
   cp .env.example .env                  # set CC_PUBLIC_HOST, WG_ADDRESS, AUTH_OUTPOST, TZ
   mkdir -p secrets && openssl rand -hex 32 > secrets/cc-secret-key && chmod 600 secrets/cc-secret-key
   docker compose up -d --build
   ```
5. **Certificates.** `deploy/Caddyfile` starts with `tls internal` (trust Caddy's
   root CA on your machines). For a public certificate without opening port 80, use
   the DNS challenge with your DNS provider's Caddy module.

Keep `secrets/cc-secret-key` out of the backups of the `cc-data` volume: the key
and the encrypted `secrets.enc` together unlock every stored API key.

## Moving from the desktop app

- **Data:** copy `command-center.db` (and the `*-hidden-*.json` files) from the
  desktop app's data folder (`%APPDATA%\com.james.commandcenter` on Windows) into
  the `cc-data` volume while the server is stopped.
- **Keys:** paste the Claude, Atera and UniFi keys again in Settings > Connections.
  They are not copied out of Windows Credential Manager.
- **Microsoft 365:** reconnect each inbox. On the server, sign-in shows a code to
  enter at microsoft.com/devicelogin from any device. In the Entra app
  registration, turn on *Authentication > Allow public client flows*.

## Local models

Once the GPU is in, install the NVIDIA driver and the NVIDIA container toolkit,
then `docker compose --profile llm up -d` starts Ollama next to the server. The
plan assumes at least 8 GB of VRAM: 7–8B models for triage and summaries, and
Whisper for voice. Wiring agents to pick Claude or a local model is the next step.

## Settings reference (`cc-server`)

| Variable | Default | Meaning |
|---|---|---|
| `CC_BIND` | `127.0.0.1:8484` | Listen address. Anything but loopback requires one of the two auth settings. |
| `CC_AUTH_HEADER` | unset | Header the sign-in proxy sets for signed-in users (`X-Authentik-Username`). |
| `CC_API_TOKEN` | unset | Also accept `Authorization: Bearer <token>` (scripts). |
| `CC_DATA_DIR` | `./data` | Database, encrypted secrets, hidden-site lists. |
| `CC_WEB_DIR` | `./dist` | The built frontend. |
| `CC_SECRET_KEY_FILE` | unset | 32-byte key (hex, base64 or raw). systemd `LoadCredential=cc-secret-key` also works. |
| `TZ` | host | Time zone for reminders and agent context. |
