# Running Command Center on a server

The target setup (see the architecture plan in the project) is one Linux box that
runs the backend and serves the web UI on a public name, so any browser, on your
network or off it, is the command center. Nothing reaches the app without signing
in to your own Authentik with MFA or a passkey. WireGuard stays as the admin path.

```
any browser ── HTTPS (public IP block, NAT 443) ──▶ Caddy ──▶ Authentik sign-in (MFA / passkey)
                                                      │
                                                      └──▶ cc-server ──▶ M365, Atera, UniFi, ESXi…
admin laptop ── WireGuard ──▶ SSH, Authentik admin UI, the same app
```

## What faces the internet

| Port | What | Why |
|---|---|---|
| TCP 443 | Caddy: `CC_PUBLIC_HOST` (the app) and `CC_AUTH_HOST` (Authentik's sign-in pages) | The app itself |
| TCP 80 | Caddy | Let's Encrypt certificates; everything else redirects to HTTPS |
| UDP (your choice) | WireGuard | Admin path |

Nothing else. In particular:

- **Every app path needs a signed-in session**, including `/api/call/*`, the
  `/api/events` WebSocket and `/api/health`. cc-server is never published; it
  also refuses cross-site browser requests and any request without the identity
  header Caddy sets after sign-in.
- **Authentik's admin UI** (`/if/admin/`) answers only from `ADMIN_RANGES` (the
  WireGuard subnet). Sign-in pages stay public because sign-in needs them.
- **SSH** listens only on the WireGuard address.
- **Docker skips ufw** for published ports, which is why Compose binds Caddy to
  `PUBLIC_ADDRESS` and `WG_ADDRESS` explicitly rather than `0.0.0.0`.

## Try it on a Linux VM

Ubuntu Server 24.04 with 4 vCPU, 8 GB RAM and 60 GB of disk is enough until the
GPU box exists (add 16 GB of RAM and 50 GB of disk if you run local models on the
CPU; see *Local AI* below). Install Docker (`curl -fsSL https://get.docker.com | sh`).

1. **Names.** Pick an address on your public IP block and NAT TCP 80 and 443 on it
   to the server's LAN address (`PUBLIC_ADDRESS`). In public DNS, point
   `CC_PUBLIC_HOST` and `CC_AUTH_HOST` (for example `cc.` and `auth.` under your
   domain) at that public address. If you want the same names to work at home
   without hairpin NAT, add split-horizon records pointing at `PUBLIC_ADDRESS`.
2. **WireGuard (admin path).** Install `wireguard-tools`, create `wg0` with
   `10.66.0.1/24`, add a peer per admin machine, and forward its UDP port. Set
   `ListenAddress` in `/etc/ssh/sshd_config` to `10.66.0.1`.
3. **Authentik.** Run it from its own compose file (goauthentik.io), bound to
   `10.66.0.1:9000` rather than the public address, with `AUTHENTIK_HOST` set to
   `https://<CC_AUTH_HOST>`. Then, from a WireGuard machine:
   - Create a Proxy Provider in *forward auth (single application)* mode with
     external host `https://<CC_PUBLIC_HOST>`, an Application for it, and add
     it to the embedded outpost.
   - **Require MFA:** in the default authentication flow, set the
     authenticator validation stage's *Not configured action* to force setup,
     and allow WebAuthn (passkeys) and TOTP. Enroll a passkey on each device and
     keep the recovery codes offline.
   - **Shorten sessions:** set the provider's token validity (for example 12
     hours) and turn on *Remember me* only if you want it.
   - **Slow down guessing:** add a Reputation policy to the identification and
     password stages, and turn off self-enrollment and password recovery flows
     you don't use.
   - Bind the application to your user or a group so a new account can't use
     the app by default.
4. **Command Center.**
   ```bash
   cd deploy
   cp .env.example .env                  # names, addresses, ACME_EMAIL, TZ
   mkdir -p secrets && openssl rand -hex 32 > secrets/cc-secret-key && chmod 600 secrets/cc-secret-key
   docker compose up -d --build
   ```
   Caddy gets public certificates for both names on first start.
5. **Check from outside** (phone on cellular): `https://<CC_PUBLIC_HOST>` should
   send you to Authentik, and `curl -i https://<CC_PUBLIC_HOST>/api/health`
   should redirect to sign-in rather than answer `ok`.

Keep `secrets/cc-secret-key` out of the backups of the `cc-data` volume: the key
and the encrypted `secrets.enc` together unlock every stored API key.

## Keeping it safe once it's up

- **Patch:** `docker compose pull && docker compose up -d --build` for Caddy and
  cc-server, and the same in Authentik's folder, at least monthly and whenever
  Authentik ships a security release. Unattended upgrades for the host.
- **Watch:** Caddy writes JSON access logs to `access.log` in the `caddy-data`
  volume. CrowdSec (Caddy collection) or fail2ban on those logs plus Authentik's
  login events will block repeat offenders. Authentik can notify you on each new
  sign-in.
- **Narrow it if you like:** geo-blocking at the edge, or a `remote_ip` allow
  list in the Caddyfile, cuts the noise a lot. Neither is required.
- **Go private again:** remove the two `PUBLIC_ADDRESS` lines from
  `docker-compose.yml` and the NAT rule. The app keeps working over WireGuard at
  the same name (via split-horizon DNS).

## The desktop app on the server

The desktop app and the web app are the same app. Point the desktop app at the
server and it shows the same todos, reminders, dashboard layout, agents and
connectors as any browser, live, while keeping its tray icon, global shortcut
and OS notifications. Keys stay on the server; the desktop app only holds its
own sign-in.

1. **An app password for each computer.** The desktop app can't do a browser
   sign-in with a passkey for every request, so it signs in like a script: in
   Authentik, under your user's *Settings > Tokens and App passwords*, create an
   app password named after the computer. Revoke it there if the computer is
   lost. The proxy provider must have *Intercept header authentication* on (the
   default), which lets Authentik accept `Authorization: Basic user:app-password`
   in place of a browser session. Without Authentik (a lab box), leave the
   username empty and use cc-server's `CC_API_TOKEN` instead.
2. **Settings > Command Center server** in the desktop app: enter
   `https://<CC_PUBLIC_HOST>`, your Authentik username and the app password,
   then *Save and test*. The password goes to Windows Credential Manager.
3. **Move connectors.** The same card lists every connector on this computer
   and on the server. Tick the ones to copy and choose *Copy to the server*:
   the Claude, Atera and UniFi keys, Atera hidden customers and UniFi hidden
   sites, and each signed-in Microsoft 365 inbox (its app registration and
   sign-in). Keys go from the keychain straight to the server's encrypted
   store over HTTPS; there is no API that reads a key back out. Each connector
   reports whether it works from the server.
4. **Switch this app to the server.** The app restarts on the server's data.
   *Use this computer's data instead* switches back; local data is left as it
   was. In server mode the desktop app runs no polling of its own, so alerts
   and reminders are not doubled.

If a moved Microsoft 365 inbox shows a sign-in error on the server, reconnect it
from the Inbox page: on the server, sign-in shows a code to enter at
microsoft.com/devicelogin from any device. In the Entra app registration, turn on
*Authentication > Allow public client flows*.

### Moving todos, reminders and agents

Connectors move from the app. For the rest of the data, copy
`command-center.db` from the desktop app's data folder
(`%APPDATA%\com.james.commandcenter` on Windows) into the `cc-data` volume while
the server is stopped and before you first use it, since this replaces the
server's data.

## Local AI (Ollama)

Ollama runs next to cc-server and gives the app a **Local AI** provider: make an
agent with it, or type `local: …` in the ask box. Prompts and answers never leave
the server. Ollama is not published and not in the Caddyfile; only cc-server
reaches it, over the compose network, so it needs no key of its own.

### Where it runs

| Option | Notes |
|---|---|
| **Separate box** (the plan's choice) | Ubuntu Server 24.04 on its own hardware with an NVIDIA card. The command center keeps working when the ESXi host it manages is down or being patched. |
| **VM on the ESXi host** | Works with the card passed through to the VM: on the host, *Manage > Hardware > PCI devices*, toggle passthrough for the GPU (and its audio function) and reboot the host. Give the VM EFI firmware, reserve all of its memory, then add the GPU as a PCI device. Consumer GeForce cards may also need `hypervisor.cpuid.v0 = FALSE` in the VM's advanced settings. The GPU then belongs to that VM only. |
| **No GPU yet** | Ollama runs on the CPU. 3–4B models answer in seconds to a minute; anything larger is slow. |

Plan for 8 GB of GPU memory or more. Rough guide to what fits:

| GPU memory | Models to start with |
|---|---|
| none (CPU) | `llama3.2:3b`, `qwen3:4b` |
| 8 GB | `qwen3:8b`, `llama3.1:8b` |
| 16 GB | `qwen3:14b`, `gemma3:12b` |
| 24 GB | `qwen3:30b-a3b`, `gemma3:27b` |

### Turn it on

1. **GPU driver** (skip on CPU). On Ubuntu 24.04: `sudo ubuntu-drivers install`,
   reboot, and check `nvidia-smi` lists the card.
2. **NVIDIA container toolkit** (skip on CPU), so containers can use the card:
   ```bash
   curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | sudo gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg
   curl -fsSL https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list \
     | sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#' \
     | sudo tee /etc/apt/sources.list.d/nvidia-container-toolkit.list
   sudo apt-get update && sudo apt-get install -y nvidia-container-toolkit
   sudo nvidia-ctk runtime configure --runtime=docker && sudo systemctl restart docker
   docker run --rm --gpus all ubuntu nvidia-smi   # should list the card
   ```
3. **`.env`:** `COMPOSE_PROFILES=llm` starts Ollama with the stack (it is already
   set in `.env.example`). With a GPU, also set
   `COMPOSE_FILE=docker-compose.yml:docker-compose.gpu.yml`. Then
   `docker compose up -d`.
4. **Pull a model:** `docker compose exec ollama ollama pull qwen3:8b`. Models
   live in the `ollama` volume, so allow 5–20 GB of disk per model.
   `docker compose exec ollama ollama ps` shows whether a model is on the GPU.
5. **In the app:** *Settings > Connections > Local AI (Ollama)* lists the pulled
   models. Pick one in an agent's model list, or use `local: …` from the ask box,
   which uses the smallest pulled model.

Ollama keeps a model loaded for 30 minutes after use and holds one at a time
(`OLLAMA_KEEP_ALIVE` and `OLLAMA_MAX_LOADED_MODELS` in `docker-compose.yml`), so
the first ask after a quiet spell waits while the model loads. The desktop app
in local mode uses Ollama on that computer (`http://127.0.0.1:11434`) if one is
running; set `CC_OLLAMA_URL` to point either app elsewhere.

Whisper for voice is still to come; it will sit next to Ollama the same way.

## Settings reference (`cc-server`)

| Variable | Default | Meaning |
|---|---|---|
| `CC_BIND` | `127.0.0.1:8484` | Listen address. Anything but loopback requires one of the two auth settings. |
| `CC_AUTH_HEADER` | unset | Header the sign-in proxy sets for signed-in users (`X-Authentik-Username`). |
| `CC_API_TOKEN` | unset | Also accept `Authorization: Bearer <token>` (scripts). |
| `CC_DATA_DIR` | `./data` | Database, encrypted secrets, hidden-site lists. |
| `CC_WEB_DIR` | `./dist` | The built frontend. |
| `CC_SECRET_KEY_FILE` | unset | 32-byte key (hex, base64 or raw). systemd `LoadCredential=cc-secret-key` also works. |
| `CC_OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama for the Local AI provider. Compose sets `http://ollama:11434`. |
| `TZ` | host | Time zone for reminders and agent context. |
