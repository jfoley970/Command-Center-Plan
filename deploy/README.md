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
GPU box exists. Install Docker (`curl -fsSL https://get.docker.com | sh`).

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
