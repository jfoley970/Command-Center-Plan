//! cc-server: runs the Command Center backend on a server and serves the web UI.
//!
//! It is meant to sit behind a reverse proxy (Caddy) that terminates TLS and
//! hands sign-in to the local auth service. The proxy may face the internet, so
//! the server itself trusts nothing by default:
//!
//! - `CC_AUTH_HEADER` (for example `X-Authentik-Username`): every API request
//!   must carry this header, which the proxy sets after sign-in and strips
//!   from client requests.
//! - `CC_API_TOKEN`: requests may instead carry `Authorization: Bearer <token>`
//!   (scripts, health checks from other machines).
//! - With neither set it only listens on loopback.
//! - Browser requests from another site are refused (their `Origin` must
//!   match the `Host` they were sent to), on the API and the WebSocket alike.
//!
//! `POST /hooks/pocket` is the one route without sign-in: Pocket's servers call
//! it, and each request must carry a valid Pocket HMAC signature instead. It is
//! off until a webhook secret is saved in the Pocket connector.
//!
//! Other settings: `CC_BIND` (default 127.0.0.1:8484), `CC_DATA_DIR` (default
//! ./data), `CC_WEB_DIR` (the built frontend, default ./dist), and
//! `CC_SECRET_KEY_FILE` (32-byte key for the secret store; systemd's
//! `LoadCredential=cc-secret-key` is picked up automatically).

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Path, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use cc_core::{secrets::FileStore, Core, Secrets, SignIn};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct Auth {
    header: Option<String>,
    token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    core: Arc<Core>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The secret store key: an explicit file, a systemd credential, or (with a
/// warning) one generated next to the data.
fn secret_key(data_dir: &FsPath) -> Result<[u8; 32], String> {
    if let Some(path) = env("CC_SECRET_KEY_FILE") {
        return FileStore::read_key_file(FsPath::new(&path));
    }
    if let Some(dir) = env("CREDENTIALS_DIRECTORY") {
        let path = PathBuf::from(dir).join("cc-secret-key");
        if path.exists() {
            return FileStore::read_key_file(&path);
        }
    }
    let path = data_dir.join("secret.key");
    if path.exists() {
        eprintln!("warning: using {} as the secret key. Keep it out of backups of the data folder, or set CC_SECRET_KEY_FILE.", path.display());
        return FileStore::read_key_file(&path);
    }
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let key = FileStore::generate_key();
    cc_core::secrets::write_private(&path, &key)?;
    eprintln!("warning: generated a new secret key at {}. Move it out of the data folder and set CC_SECRET_KEY_FILE.", path.display());
    Ok(key)
}

fn router(core: Arc<Core>, auth: Auth, web_dir: &FsPath) -> Router {
    let api = Router::new()
        .route("/call/{command}", post(call))
        .route("/events", get(events))
        .layer(middleware::from_fn_with_state(auth, require_auth))
        .route("/health", get(|| async { "ok" }));
    // Webhooks prove who they are with a signature, not a sign-in. A long
    // recording's transcript can be a few megabytes.
    let hooks = Router::new().route("/pocket", post(pocket_hook)).layer(DefaultBodyLimit::max(16 * 1024 * 1024));
    let spa = ServeDir::new(web_dir).fallback(ServeFile::new(web_dir.join("index.html")));
    Router::new().nest("/api", api).nest("/hooks", hooks).fallback_service(spa).with_state(AppState { core })
}

async fn require_auth(State(auth): State<Auth>, req: Request, next: Next) -> Response {
    let headers = req.headers();
    let by_proxy = auth
        .header
        .as_deref()
        .is_some_and(|h| headers.get(h).and_then(|v| v.to_str().ok()).is_some_and(|v| !v.trim().is_empty()));
    let by_token = auth.token.as_deref().is_some_and(|t| {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|given| constant_time_eq(given.trim().as_bytes(), t.as_bytes()))
    });
    let open = auth.header.is_none() && auth.token.is_none(); // Loopback only; checked at start-up.
    if by_proxy || by_token || open {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "Sign in first.").into_response()
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `POST /api/call/<command>` with the JSON arguments the frontend would pass to Tauri.
async fn call(State(s): State<AppState>, Path(command): Path<String>, headers: HeaderMap, body: String) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    // Requiring a JSON content type means a cross-site form can't make this request
    // without a CORS preflight, which this server never approves.
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.").into_response();
    }
    let args: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Bad JSON: {e}") }))).into_response(),
        }
    };
    match cc_core::call(&s.core, &command, args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// `POST /hooks/pocket`: a recording event from Pocket.
async fn pocket_hook(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let (ts, sig) = (header("x-heypocket-timestamp"), header("x-heypocket-signature"));
    match cc_core::pocket::webhook(&s.core, ts, sig, &body) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(cc_core::pocket::HookError::Disabled) => StatusCode::NOT_FOUND.into_response(),
        Err(cc_core::pocket::HookError::BadSignature) => (StatusCode::UNAUTHORIZED, "Bad signature.").into_response(),
        Err(cc_core::pocket::HookError::BadBody(e)) => (StatusCode::BAD_REQUEST, format!("Bad JSON: {e}")).into_response(),
    }
}

/// Browsers send `Origin` on WebSockets and on cross-site POSTs; only accept our
/// own page. Requests without it (scripts, same-origin GETs) pass on to sign-in.
fn same_origin(headers: &HeaderMap) -> bool {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match (origin, host) {
        (Some(origin), Some(host)) => origin.split("://").nth(1).unwrap_or(origin).eq_ignore_ascii_case(host),
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// `GET /api/events`: a WebSocket that streams every backend event as JSON.
async fn events(State(s): State<AppState>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site connection refused.").into_response();
    }
    ws.on_upgrade(move |socket| stream_events(socket, s.core))
}

async fn stream_events(mut socket: WebSocket, core: Arc<Core>) {
    let mut rx = core.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let text = serde_json::to_string(&ev).unwrap_or_default();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Fell behind: the client reloads everything on "resync".
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let text = json!({ "name": "resync", "payload": null }).to_string();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("cc-server: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let data_dir = PathBuf::from(env("CC_DATA_DIR").unwrap_or_else(|| "data".into()));
    let web_dir = PathBuf::from(env("CC_WEB_DIR").unwrap_or_else(|| "dist".into()));
    let bind: SocketAddr = env("CC_BIND")
        .unwrap_or_else(|| "127.0.0.1:8484".into())
        .parse()
        .map_err(|e| format!("CC_BIND is not an address: {e}"))?;
    let auth = Auth { header: env("CC_AUTH_HEADER"), token: env("CC_API_TOKEN") };
    if auth.header.is_none() && auth.token.is_none() && !bind.ip().is_loopback() {
        return Err("Refusing to listen beyond this machine without auth. Set CC_AUTH_HEADER (behind the sign-in proxy) or CC_API_TOKEN.".into());
    }

    std::fs::create_dir_all(&data_dir).map_err(|e| format!("Could not create {}: {e}", data_dir.display()))?;
    let store = FileStore::open(&data_dir.join("secrets.enc"), secret_key(&data_dir)?)?;
    let core = Core::open(&data_dir, Secrets::new(store), SignIn::DeviceCode)?;
    cc_core::start_background(&core);

    let app = router(core, auth, &web_dir);
    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| format!("Could not listen on {bind}: {e}"))?;
    println!("cc-server listening on http://{bind} (data: {}, web: {})", data_dir.display(), web_dir.display());
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app(auth: Auth) -> (Router, PathBuf) {
        let dir = std::env::temp_dir().join(format!("cc-server-{}-{}", std::process::id(), rand_suffix()));
        let core = Core::open(&dir, Secrets::new(cc_core::secrets::MemoryStore::default()), SignIn::DeviceCode).unwrap();
        (router(core, auth, &dir), dir)
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn post(path: &str) -> axum::http::request::Builder {
        axum::http::Request::post(path).header(header::CONTENT_TYPE, "application/json")
    }

    async fn body_json(resp: Response) -> Value {
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[tokio::test]
    async fn call_runs_commands_and_reports_errors() {
        let (app, dir) = app(Auth { header: None, token: None });
        let body = r#"{"todo":{"title":"Patch ESXi"}}"#;
        let resp = app.clone().oneshot(post("/api/call/add_todo").body(Body::from(body)).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["title"], "Patch ESXi");

        let resp = app.clone().oneshot(post("/api/call/list_todos").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(body_json(resp).await.as_array().unwrap().len(), 1);

        let resp = app.clone().oneshot(post("/api/call/nope").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(body_json(resp).await["error"].as_str().unwrap().contains("Unknown command"));

        let form = axum::http::Request::post("/api/call/list_todos")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(form).await.unwrap().status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn cross_site_calls_are_refused() {
        let (app, dir) = app(Auth { header: None, token: None });
        let from = |origin: &str| {
            post("/api/call/list_todos")
                .header(header::HOST, "cc.example.com")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(app.clone().oneshot(from("https://cc.example.com")).await.unwrap().status(), StatusCode::OK);
        assert_eq!(app.clone().oneshot(from("https://evil.example")).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(app.oneshot(from("https://cc.example.com.evil.example")).await.unwrap().status(), StatusCode::FORBIDDEN);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn auth_header_or_token_required_when_configured() {
        let (app, dir) = app(Auth { header: Some("X-Authentik-Username".into()), token: Some("s3cret".into()) });
        let req = || post("/api/call/list_todos");
        assert_eq!(app.clone().oneshot(req().body(Body::empty()).unwrap()).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let with_header = req().header("X-Authentik-Username", "james").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(with_header).await.unwrap().status(), StatusCode::OK);
        let with_token = req().header(header::AUTHORIZATION, "Bearer s3cret").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(with_token).await.unwrap().status(), StatusCode::OK);
        let wrong_token = req().header(header::AUTHORIZATION, "Bearer nope").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(wrong_token).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        // Health stays open for container checks.
        let health = axum::http::Request::get("/api/health").body(Body::empty()).unwrap();
        assert_eq!(app.oneshot(health).await.unwrap().status(), StatusCode::OK);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn pocket_hook_needs_a_secret_and_a_valid_signature() {
        let dir = std::env::temp_dir().join(format!("cc-server-{}-{}", std::process::id(), rand_suffix()));
        let core = Core::open(&dir, Secrets::new(cc_core::secrets::MemoryStore::default()), SignIn::DeviceCode).unwrap();
        // Sign-in is on, but Pocket can't sign in; the signature is what counts.
        let app = router(core.clone(), Auth { header: Some("X-Authentik-Username".into()), token: None }, &dir);
        let body = r#"{"event":"summary.completed","recording":{"id":"r1","recordingAt":"2099-01-01T00:00:00Z"},"transcript":[{"text":"Command center, todo test the hook"}]}"#;
        let hook = |ts: String, sig: &str| {
            post("/hooks/pocket")
                .header("X-HeyPocket-Timestamp", ts)
                .header("X-HeyPocket-Signature", sig)
                .body(Body::from(body))
                .unwrap()
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis().to_string();
        assert_eq!(app.clone().oneshot(hook(now.clone(), "00")).await.unwrap().status(), StatusCode::NOT_FOUND);

        cc_core::call(&core, "pocket_set_webhook_secret", json!({ "key": "whsec_test" })).await.unwrap();
        assert_eq!(app.clone().oneshot(hook(now.clone(), "00")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let sig = hmac_hex("whsec_test", &format!("{now}.{body}"));
        assert_eq!(app.oneshot(hook(now, &sig)).await.unwrap().status(), StatusCode::NO_CONTENT);

        // The todo is added in the background.
        for _ in 0..50 {
            let todos = cc_core::call(&core, "list_todos", Value::Null).await.unwrap();
            if todos.as_array().unwrap().iter().any(|t| t["title"] == "Test the hook") {
                std::fs::remove_dir_all(dir).unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the webhook's todo never appeared");
    }

    fn hmac_hex(key: &str, msg: &str) -> String {
        // Same construction as cc-core's; written out so the test doesn't trust it.
        use sha2::{Digest, Sha256};
        let mut k = [0u8; 64];
        k[..key.len()].copy_from_slice(key.as_bytes());
        let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
        let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
        let inner = Sha256::new().chain_update(&ipad).chain_update(msg).finalize();
        Sha256::new().chain_update(&opad).chain_update(inner).finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}
