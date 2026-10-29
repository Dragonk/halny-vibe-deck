//! Remote-access HTTPS server (Nexus `serve` parity).
//!
//! Security model:
//! - the local app server ALWAYS binds 127.0.0.1 and never asks for a
//!   login — starting Halny Vibe Deck locally goes straight into the UI;
//! - remote access is a SEPARATE HTTPS listener (self-signed certificate,
//!   generated per-install and cached in the config directory) with a
//!   styled `/login` page, HttpOnly+Secure cookie sessions and a per-IP
//!   brute-force guard;
//! - credential editing (login / password / port) is LOCAL-ONLY: every
//!   request that arrives through this listener is marked with the
//!   `x-vd-origin: remote` header and `api_set_remote` rejects such
//!   requests.

use axum::{
    extract::{connect_info::Connected, ConnectInfo, Request},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::Duration,
};
use rustls_pki_types::pem::PemObject;
use tokio::io::{AsyncRead, AsyncWrite};

const COOKIE_NAME: &str = "mistraldeck_token";
const COOKIE_MAX_AGE: u64 = 30 * 24 * 60 * 60; // 30 days, sliding
const MAX_ATTEMPTS: u32 = 8;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(10 * 60);

// ---------------------------------------------------------------------------
// sessions & brute-force guard
// ---------------------------------------------------------------------------

type LoginKey = (IpAddr, String);
fn attempts() -> &'static std::sync::Mutex<HashMap<LoginKey, (u32, std::time::Instant)>> {
    static S: OnceLock<std::sync::Mutex<HashMap<LoginKey, (u32, std::time::Instant)>>> =
        OnceLock::new();
    S.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Returns true when the IP is currently rate-limited (and records the try).
/// Stable v1.0.36: the guard counts FAILED logins only and is keyed on
/// (ip, username), so a device that signs in repeatedly is never locked out
/// and a single noisy IP cannot lock an unrelated account.
fn login_blocked(ip: IpAddr, username: &str) -> bool {
    let mut map = attempts().lock().unwrap();
    let now = std::time::Instant::now();
    map.retain(|_, e| e.1 + ATTEMPT_WINDOW > now);
    let key = (ip, username.to_ascii_lowercase());
    map.get(&key).map(|e| e.0 >= MAX_ATTEMPTS).unwrap_or(false)
}

/// Record one failed attempt for (ip, username).
fn note_failed_login(ip: IpAddr, username: &str) {
    let mut map = attempts().lock().unwrap();
    let now = std::time::Instant::now();
    let key = (ip, username.to_ascii_lowercase());
    match map.get_mut(&key) {
        Some(entry) if entry.1 + ATTEMPT_WINDOW > now => entry.0 += 1,
        _ => {
            map.insert(key, (1, now));
        }
    }
}

/// Clear the counter after a successful sign-in.
fn clear_failed_logins(ip: IpAddr, username: &str) {
    attempts()
        .lock()
        .unwrap()
        .remove(&(ip, username.to_ascii_lowercase()));
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(COOKIE_NAME) {
            let rest = rest.strip_prefix('=')?;
            return Some(rest.trim().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// login page (i18n like the Nexus serve page; the language cannot come from
// the app config because the user is not logged in yet: the page prefers
// localStorage (set by the logged-in app), then navigator.languages)
// ---------------------------------------------------------------------------

const LOGIN_CSS: &str = r#"
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; min-height:100vh; min-height:100dvh; display:flex; align-items:center; justify-content:center;
    background:#111115; color:#e8e8ef; font-family:'Segoe UI',system-ui,sans-serif;
    padding:16px; padding-left:max(16px, env(safe-area-inset-left)); padding-right:max(16px, env(safe-area-inset-right)); }
  .card { width:min(340px, 100%); padding:36px 34px; background:#1a1a22; border:1px solid #2a2a36;
    border-radius:14px; box-shadow:0 12px 40px rgba(0,0,0,.5); }
  .lock { text-align:center; font-size:26px; margin-bottom:10px; opacity:.85; }
  h1 { margin:0 0 6px; font-size:19px; font-weight:700; text-align:center; letter-spacing:.3px; }
  .flame { color:#fa500f; font-weight:800; margin-right:2px; }
  .sub { margin:0 0 24px; color:#8a8a98; font-size:12.5px; line-height:1.5; text-align:center; }
  label { display:block; margin:14px 0 6px; font-size:12px; color:#a0a0b0; }
  input { width:100%; padding:10px 12px; background:#111115; border:1px solid #2a2a36;
    border-radius:8px; color:#e8e8ef; font-size:16px; outline:none; }
  input:focus { border-color:#fa500f; }
  button { margin-top:22px; width:100%; padding:11px; background:#fa500f; color:#fff;
    border:none; border-radius:8px; font-size:16px; font-weight:600; cursor:pointer; }
  button:hover { background:#ff6a2b; }
  button:disabled { opacity:.6; cursor:default; }
  .err { margin-top:14px; color:#ff6b6b; font-size:12.5px; min-height:16px; text-align:center; }
  .disc { margin-top:22px; color:#5c5c68; font-size:10.5px; line-height:1.5; text-align:center; }
"#;

const LOGIN_JS: &str = r#"
  (function(){
    var DICT = {
      en: { sub: "Sign in to access the hosted instance.",
            user: "Username", pass: "Password", login: "Sign in",
            invalid: "Invalid username or password.",
            tooMany: "Too many attempts - try again later.",
            conn: "Connection error.",
            disc: "Unofficial community project — not affiliated with Mistral AI." },
      pl: { sub: "Zaloguj się, aby uzyskać dostęp do hostowanej instancji.",
            user: "Login", pass: "Hasło", login: "Zaloguj się",
            invalid: "Nieprawidłowy login lub hasło.",
            tooMany: "Zbyt wiele prób - spróbuj ponownie później.",
            conn: "Błąd połączenia.",
            disc: "Nieoficjalny projekt społecznościowy — niepowiązany z Mistral AI." },
      fr: { sub: "Connectez-vous pour accéder à l'instance hébergée.",
            user: "Identifiant", pass: "Mot de passe", login: "Se connecter",
            invalid: "Identifiant ou mot de passe incorrect.",
            tooMany: "Trop de tentatives - réessayez plus tard.",
            conn: "Erreur de connexion.",
            disc: "Projet communautaire non officiel — non affilié à Mistral AI." },
      es: { sub: "Inicia sesión para acceder a la instancia alojada.",
            user: "Usuario", pass: "Contraseña", login: "Iniciar sesión",
            invalid: "Usuario o contraseña incorrectos.",
            tooMany: "Demasiados intentos - inténtalo de nuevo más tarde.",
            conn: "Error de conexión.",
            disc: "Proyecto comunitario no oficial — no afiliado a Mistral AI." },
      de: { sub: "Melde dich an, um auf die gehostete Instanz zuzugreifen.",
            user: "Benutzername", pass: "Passwort", login: "Anmelden",
            invalid: "Ungültiger Benutzername oder falsches Passwort.",
            tooMany: "Zu viele Versuche - versuche es später erneut.",
            conn: "Verbindungsfehler.",
            disc: "Inoffizielles Community-Projekt — nicht mit Mistral AI verbunden." },
      it: { sub: "Accedi per utilizzare l'istanza ospitata.",
            user: "Nome utente", pass: "Password", login: "Accedi",
            invalid: "Nome utente o password non validi.",
            tooMany: "Troppi tentativi - riprova più tardi.",
            conn: "Errore di connessione.",
            disc: "Progetto comunitario non ufficiale — non affiliato a Mistral AI." }
    };
    var want = null;
    try { want = localStorage.getItem('vdLang'); } catch (err) {}
    var nav = (navigator.languages || [navigator.language || 'en']);
    var cur = null;
    var prefs = [want].concat(Array.prototype.slice.call(nav));
    for (var i = 0; i < prefs.length; i++) {
      var base = String(prefs[i] || '').toLowerCase().split('-')[0];
      if (DICT[base]) { cur = base; break; }
    }
    if (!cur) cur = 'en';
    document.documentElement.lang = cur;
    var d = DICT[cur];
    document.getElementById('sub').textContent = d.sub;
    document.getElementById('lu').textContent = d.user;
    document.getElementById('lp').textContent = d.pass;
    document.getElementById('b').textContent = d.login;
    document.getElementById('disc').textContent = d.disc;
    document.getElementById('f').addEventListener('submit', function(ev){
      ev.preventDefault();
      var b = document.getElementById('b'); b.disabled = true;
      fetch('/api/auth/login',{method:'POST',headers:{'Content-Type':'application/json'},
        body:JSON.stringify({username:document.getElementById('u').value,
        password:document.getElementById('p').value})})
      .then(function(r){
        if (r.ok) { location.href='/'; return; }
        b.disabled = false;
        document.getElementById('e').textContent =
          r.status === 429 ? d.tooMany : d.invalid;
      }).catch(function(){ b.disabled=false;
        document.getElementById('e').textContent=d.conn; });
    });
  })();
"#;

fn login_page_html() -> String {
    let mut s = String::with_capacity(12 * 1024);
    s.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n");
    s.push_str("<title>Halny Vibe Deck</title>\n<style>");
    s.push_str(LOGIN_CSS);
    s.push_str("\n</style>\n</head>\n<body>\n<div class=\"card\">\n");
    s.push_str("  <div class=\"lock\">&#128274;</div>\n");
    s.push_str("  <h1>Halny Vibe Deck</h1>\n");
    s.push_str("  <div class=\"sub\" id=\"sub\"></div>\n");
    s.push_str("  <form id=\"f\" autocomplete=\"off\">\n");
    s.push_str("    <label for=\"u\" id=\"lu\"></label>\n");
    s.push_str("    <input id=\"u\" name=\"username\" type=\"text\" autocomplete=\"username\" required>\n");
    s.push_str("    <label for=\"p\" id=\"lp\"></label>\n");
    s.push_str("    <input id=\"p\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required>\n");
    s.push_str("    <button id=\"b\" type=\"submit\"></button>\n");
    s.push_str("    <div class=\"err\" id=\"e\"></div>\n");
    s.push_str("  </form>\n");
    s.push_str("  <div class=\"disc\" id=\"disc\"></div>\n");
    s.push_str("</div>\n<script>");
    s.push_str(LOGIN_JS);
    s.push_str("\n</script>\n</body>\n</html>");
    s
}

// ---------------------------------------------------------------------------
// routes
// ---------------------------------------------------------------------------

async fn login_page(headers: HeaderMap) -> Response {
    if let Some(tok) = cookie_token(&headers) {
        if mistraldeck_engine::Engine::global().check_token(&tok) {
            return Redirect::to("/").into_response();
        }
    }
    Html(login_page_html()).into_response()
}

#[derive(serde::Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

fn set_cookie_header(token: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}"
    ))
    .unwrap_or(HeaderValue::from_static(""))
}

async fn login_post(
    ConnectInfo(addr): ConnectInfo<RemoteConnectInfo>,
    axum::Json(body): axum::Json<LoginBody>,
) -> Response {
    if login_blocked(addr.0.ip(), &body.username) {
        return json_status(StatusCode::TOO_MANY_REQUESTS, r#"{"ok":false,"error":"Too many attempts"}"#);
    }
    match mistraldeck_engine::Engine::global().login(&body.username, &body.password) {
        Some(token) => {
            clear_failed_logins(addr.0.ip(), &body.username);
            let mut resp = json_status(StatusCode::OK, r#"{"ok":true}"#);
            resp.headers_mut().insert(
                header::SET_COOKIE,
                set_cookie_header(&token, COOKIE_MAX_AGE),
            );
            resp
        }
        None => {
            note_failed_login(addr.0.ip(), &body.username);
            json_status(StatusCode::UNAUTHORIZED, r#"{"ok":false,"error":"Invalid credentials"}"#)
        }
    }
}

async fn logout_post(headers: HeaderMap) -> Response {
    if let Some(tok) = cookie_token(&headers) {
        mistraldeck_engine::Engine::global().logout(&tok);
    }
    let mut resp = json_status(StatusCode::OK, r#"{"ok":true}"#);
    resp.headers_mut()
        .insert(header::SET_COOKIE, set_cookie_header("", 0));
    resp
}

fn json_status(status: StatusCode, body: &'static str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

/// Auth gate for everything routed through the remote HTTPS listener:
/// unauthenticated browsers are redirected to `/login`, unauthenticated
/// API / websocket calls get a plain 401. Authenticated requests are
/// marked `x-vd-origin: remote` so local-only APIs can refuse them.
async fn remote_auth(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    // PWA statics are reachable without a session (service-worker updates
    // must not depend on cookie state; Nexus serves them pre-auth too).
    if path == "/login"
        || path.starts_with("/api/auth/")
        || path == "/sw.js"
        || path == "/manifest.webmanifest"
        || path == "/icon-256.png"
    {
        return next.run(req).await;
    }
    let token = cookie_token(req.headers());
    let ok = token
        .as_deref()
        .map(|t| mistraldeck_engine::Engine::global().check_token(t))
        .unwrap_or(false);
    if !ok {
        let accepts_html = req
            .headers()
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/html"))
            .unwrap_or(false);
        if accepts_html {
            return Redirect::to("/login").into_response();
        }
        return json_status(StatusCode::UNAUTHORIZED, r#"{"ok":false,"error":"Unauthorized"}"#);
    }
    let mut req = req;
    if let Ok(v) = HeaderValue::from_str("remote") {
        req.headers_mut().insert("x-vd-origin", v);
    }
    // Sliding window (stable v1.0.36): re-issue the cookie once the token is
    // past half of its lifetime, so an active device never has to log in
    // again — and a restart does not invalidate anything either.
    let refresh = token
        .as_deref()
        .map(|t| mistraldeck_engine::Engine::global().token_refresh_due(t))
        .unwrap_or(false);
    let mut resp = next.run(req).await;
    if refresh {
        if let Some(user) = token
            .as_deref()
            .and_then(|t| mistraldeck_engine::Engine::global().token_user(t))
        {
            if let Some(fresh) = mistraldeck_engine::Engine::global().login_token_for(&user) {
                resp.headers_mut()
                    .insert(header::SET_COOKIE, set_cookie_header(&fresh, COOKIE_MAX_AGE));
            }
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// self-signed certificate (generated once, cached, regenerated before expiry)
// ---------------------------------------------------------------------------

fn serve_dir() -> Option<PathBuf> {
    let cfg = mistraldeck_engine::EngineConfig::path()?;
    let parent = cfg.parent()?.to_path_buf();
    Some(parent.join("serve"))
}

fn ensure_certificate() -> Result<(String, String), String> {
    let dir = serve_dir().ok_or_else(|| "config directory unavailable".to_string())?;
    let cert_file = dir.join("cert.pem");
    let key_file = dir.join("key.pem");
    let meta_file = dir.join("cert-meta.json");
    let expires_soon =
        std::time::SystemTime::now() + std::time::Duration::from_secs(30 * 24 * 60 * 60);

    if let (Ok(cert), Ok(key), Ok(meta)) = (
        std::fs::read_to_string(&cert_file),
        std::fs::read_to_string(&key_file),
        std::fs::read_to_string(&meta_file),
    ) {
        let expires = serde_json::from_str::<serde_json::Value>(&meta)
            .ok()
            .and_then(|m| m.get("expires").and_then(|e| e.as_u64()))
            .unwrap_or(0);
        let expires = std::time::UNIX_EPOCH + std::time::Duration::from_millis(expires);
        if expires > expires_soon && !cert.is_empty() && !key.is_empty() {
            return Ok((cert, key));
        }
    }

    // Generate a fresh self-signed certificate (SANs: localhost + loopback
    // IPs; LAN access by IP still shows the usual self-signed warning).
    let rcgen::CertifiedKey { cert, key_pair } = rcgen::generate_simple_self_signed(vec![
        "Halny Vibe Deck".to_string(),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ])
    .map_err(|e| format!("self-signed certificate failed: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(&cert_file, &cert_pem).map_err(|e| format!("write cert: {e}"))?;
    std::fs::write(&key_file, &key_pem).map_err(|e| format!("write key: {e}"))?;
    let expires_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        + 365 * 24 * 60 * 60 * 1000;
    let _ = std::fs::write(
        &meta_file,
        format!("{{\"expires\":{expires_ms}}}"),
    );
    Ok((cert_pem, key_pem))
}

// ---------------------------------------------------------------------------
// TLS listener (axum Listener impl around tokio-rustls; the handshake runs
// on the first poll of the returned stream)
// ---------------------------------------------------------------------------

/// Local connect-info wrapper (the orphan rule forbids implementing
/// `Connected<IncomingStream<..>>` for the foreign SocketAddr).
#[derive(Clone, Debug)]
pub struct RemoteConnectInfo(pub SocketAddr);

impl Connected<axum::serve::IncomingStream<'_, TlsListener>> for RemoteConnectInfo {
    fn connect_info(stream: axum::serve::IncomingStream<'_, TlsListener>) -> Self {
        RemoteConnectInfo(*stream.remote_addr())
    }
}

struct TlsListener {
    tcp: tokio::net::TcpListener,
    tls: Arc<rustls::ServerConfig>,
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.tcp.accept().await {
                Ok((stream, addr)) => {
                    let accept =
                        tokio_rustls::TlsAcceptor::from(self.tls.clone()).accept(stream);
                    return (
                        TlsStream {
                            state: TlsState::Handshaking(accept),
                        },
                        addr,
                    );
                }
                Err(e) => {
                    // transient accept error (EMFILE etc.) — log and retry
                    eprintln!("mistraldeck: remote accept error: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

enum TlsState {
    Handshaking(tokio_rustls::Accept<tokio::net::TcpStream>),
    Streaming(tokio_rustls::server::TlsStream<tokio::net::TcpStream>),
}

struct TlsStream {
    state: TlsState,
}

impl AsyncRead for TlsStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                TlsState::Handshaking(accept) => {
                    match std::pin::Pin::new(accept).poll(cx) {
                        Poll::Ready(Ok(stream)) => {
                            self.state = TlsState::Streaming(stream);
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                TlsState::Streaming(stream) => {
                    return std::pin::Pin::new(stream).poll_read(cx, buf);
                }
            }
        }
    }
}

impl AsyncWrite for TlsStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                TlsState::Handshaking(accept) => {
                    match std::pin::Pin::new(accept).poll(cx) {
                        Poll::Ready(Ok(stream)) => {
                            self.state = TlsState::Streaming(stream);
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                TlsState::Streaming(stream) => {
                    return std::pin::Pin::new(stream).poll_write(cx, buf);
                }
            }
        }
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                TlsState::Handshaking(accept) => {
                    match std::pin::Pin::new(accept).poll(cx) {
                        Poll::Ready(Ok(stream)) => {
                            self.state = TlsState::Streaming(stream);
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                TlsState::Streaming(stream) => {
                    return std::pin::Pin::new(stream).poll_flush(cx);
                }
            }
        }
    }

    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                TlsState::Handshaking(accept) => {
                    match std::pin::Pin::new(accept).poll(cx) {
                        Poll::Ready(Ok(stream)) => {
                            self.state = TlsState::Streaming(stream);
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                TlsState::Streaming(stream) => {
                    return std::pin::Pin::new(stream).poll_shutdown(cx);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// entry point
// ---------------------------------------------------------------------------

/// Serve the app over HTTPS (self-signed) on `0.0.0.0:port`, with cookie
/// auth in front of everything except the `/login` page + auth endpoints.
/// Bind with a short retry loop: after an in-app update the previous
/// instance may still hold the socket for a moment (v1.0.19 parity).
async fn bind_with_retry(
    host: &str,
    port: u16,
    attempts: u32,
) -> Result<tokio::net::TcpListener, String> {
    let mut last = String::new();
    for attempt in 0..attempts.max(1) {
        match tokio::net::TcpListener::bind((host, port)).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                last = e.to_string();
                if attempt + 1 < attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                }
            }
        }
    }
    Err(format!("bind {host}:{port} failed: {last}"))
}

/// rustls config for the self-signed certificate (both listeners share it).
fn tls_server_config() -> Result<rustls::ServerConfig, String> {
    let (cert_pem, key_pem) = ensure_certificate()?;
    let certs: Vec<rustls_pki_types::CertificateDer<'static>> =
        rustls_pki_types::CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("bad certificate pem: {e}"))?;
    let key = rustls_pki_types::PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| format!("bad key pem: {e}"))?;
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("tls config failed: {e}"))
}

/// Local UI over **HTTPS** (Z14): used by the headless/server build, where a
/// browser is the client. The desktop webview keeps plain loopback HTTP —
/// webview engines refuse a self-signed certificate and wry exposes no TLS
/// hook, and that listener never leaves 127.0.0.1. Every network-exposed
/// surface is HTTPS-only either way.
pub async fn run_local_https(app_router: Router, port: u16, require_login: bool) -> Result<(), String> {
    let tls = tls_server_config()?;
    let tcp = bind_with_retry("127.0.0.1", port, 6).await?;
    let router = if require_login {
        let login = Router::new()
            .route("/login", get(login_page))
            .route("/api/auth/login", post(login_post))
            .route("/api/auth/logout", post(logout_post));
        app_router
            .layer(middleware::from_fn(remote_auth))
            .merge(login)
    } else {
        app_router
    };
    let router = router.layer(middleware::from_fn(crate::pwa::gzip_mw));
    eprintln!(
        "mistraldeck: local UI at https://127.0.0.1:{port} (self-signed certificate{})",
        if require_login { "; login required" } else { "" }
    );
    axum::serve(
        TlsListener {
            tcp,
            tls: Arc::new(tls),
        },
        router.into_make_service_with_connect_info::<RemoteConnectInfo>(),
    )
    .await
    .map_err(|e| format!("local https server failed: {e}"))
}

pub async fn run_remote(app_router: Router, port: u16) -> Result<(), String> {
    let tls = tls_server_config()?;

    // v1.0.19 parity: after an in-app update the previous instance may
    // still hold the port for a moment (socket teardown, systemd handover).
    // Retry for a few seconds before declaring the bind a failure.
    let tcp = bind_with_retry("0.0.0.0", port, 12).await?;

    let login = Router::new()
        .route("/login", get(login_page))
        .route("/api/auth/login", post(login_post))
        .route("/api/auth/logout", post(logout_post));
    let remote = app_router
        .layer(middleware::from_fn(remote_auth))
        .layer(middleware::from_fn(crate::pwa::gzip_mw))
        .merge(login);

    eprintln!(
        "mistraldeck: remote access at https://0.0.0.0:{port} (self-signed certificate; login required)"
    );
    // systemd Type=notify: the remote listener is up.
    mistraldeck_engine::systemd::notify_state("READY=1");
    mistraldeck_engine::systemd::start_watchdog();
    axum::serve(
        TlsListener {
            tcp,
            tls: Arc::new(tls),
        },
        remote.into_make_service_with_connect_info::<RemoteConnectInfo>(),
    )
    .await
    .map_err(|e| format!("remote server failed: {e}"))
}
