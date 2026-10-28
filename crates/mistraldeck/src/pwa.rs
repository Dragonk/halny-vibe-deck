//! PWA static routes + gzip middleware (Nexus `serve` / v0.13.4 parity).
//!
//! - `/sw.js`, `/manifest.webmanifest` and `/icon-256.png` are served from
//!   the binary itself (no extra files to ship next to the exe; the icon is
//!   re-encoded from the repo's `assets/icon.ico.b64` via the `image` crate).
//! - The service worker is cache-first for hashed assets and network-first
//!   for the app shell, so a new release is never pinned by a stale
//!   `index.html` (v0.7.2 fix carried over from Nexus).
//! - The gzip middleware compresses compressible responses larger than 1 KB
//!   for clients that advertise `Accept-Encoding: gzip` (v0.13.4 parity).

use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::io::{Cursor, Write};
use std::sync::OnceLock;

const SW_JS: &str = r#"/* Halny Vibe Deck service worker (serve mode / PWA).
 * Cache-first for assets, network-first for the app shell (a stale cached
 * index.html would pin users to the previous release forever); the API and
 * the websocket upgrade are never cached. */
const CACHE = 'halny-vibe-deck-shell-__VERSION__'

self.addEventListener('install', () => {
  self.skipWaiting()
})

self.addEventListener('activate', (event) => {
  // Purge caches from previous releases — a stale cached shell pins
  // users to the old build (asset names are content-hashed, so the
  // only way back is a cache that still holds the old shell/assets).
  event.waitUntil((async () => {
    const keys = await caches.keys()
    await Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    await self.clients.claim()
  })())
})

self.addEventListener('fetch', (event) => {
  const req = event.request
  if (req.method !== 'GET') return
  const url = new URL(req.url)
  if (url.origin !== self.location.origin) return
  if (url.pathname.startsWith('/api/') || url.pathname === '/sw.js' || url.pathname === '/login') return

  // The app shell (navigation) must be network-first.
  if (url.pathname === '/' || url.pathname === '/index.html') {
    event.respondWith(
      fetch(req)
        .then((res) => {
          if (res && res.ok) {
            const copy = res.clone()
            caches.open(CACHE).then((c) => c.put(req, copy))
          }
          return res
        })
        .catch(() => caches.match(req).then((hit) => hit || Response.error()))
    )
    return
  }

  event.respondWith(
    caches.match(req).then((hit) => {
      if (hit) return hit
      return fetch(req).then((res) => {
        if (res && res.ok) {
          const p = url.pathname
          const cacheable =
            p.startsWith('/assets/') ||
            p.startsWith('/wasm/') ||
            p === '/' ||
            p === '/index.html' ||
            p === '/manifest.webmanifest' ||
            p === '/icon-256.png'
          if (cacheable) {
            const copy = res.clone()
            caches.open(CACHE).then((c) => c.put(req, copy))
          }
        }
        return res
      })
    })
  )
})
"#;

const MANIFEST: &str = r##"{
  "name": "Halny Vibe Deck",
  "short_name": "HalnyVibeDeck",
  "description": "Halny Vibe Deck - client for the Mistral Vibe agent",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "orientation": "any",
  "background_color": "#111115",
  "theme_color": "#111115",
  "icons": [
    {
      "src": "/icon-256.png",
      "sizes": "256x256",
      "type": "image/png",
      "purpose": "any"
    },
    {
      "src": "/icon-256.png",
      "sizes": "256x256",
      "type": "image/png",
      "purpose": "maskable"
    }
  ]
}"##;

// ---------------------------------------------------------------------------
// icon: decode the repo's base64 ICO and re-encode the largest frame as PNG
// ---------------------------------------------------------------------------

static ICON_PNG: OnceLock<Vec<u8>> = OnceLock::new();

fn icon_png() -> &'static [u8] {
    ICON_PNG.get_or_init(|| {
        let ico = decode_base64(include_str!("../assets/icon.ico.b64").trim());
        if ico.is_empty() {
            return Vec::new();
        }
        let img = match image::load_from_memory(&ico) {
            Ok(img) => img,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        let ok = img
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .is_ok();
        if ok {
            out
        } else {
            Vec::new()
        }
    })
}

/// Minimal standard-alphabet base64 decoder (same as the build script).
pub(crate) fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        let v = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' | b' ' => continue,
            b'=' => break,
            _ => continue,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// routes
// ---------------------------------------------------------------------------

fn static_response(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut resp = (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response();
    // Root static files revalidate on every load (v0.13.4: only the
    // content-hashed asset bundles under /assets/ are cache-forever, and
    // those are already handled by the dioxus asset server).
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

async fn serve_sw_js() -> Response {
    // inject the crate version into the cache name so every release
    // gets a fresh cache and the activate handler purges the old one
    let sw = SW_JS.replace("__VERSION__", env!("CARGO_PKG_VERSION"));
    static_response("application/javascript", sw.into_bytes())
}

async fn serve_manifest() -> Response {
    static_response(
        "application/manifest+json",
        MANIFEST.as_bytes().to_vec(),
    )
}

async fn serve_icon_png() -> Response {
    let png = icon_png().to_vec();
    if png.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
            "icon unavailable",
        )
            .into_response();
    }
    static_response("image/png", png)
}

/// Add the PWA static routes to the app router. Explicit routes win over
/// the dioxus SSR fallback, and the files ship inside the binary — no
/// `public/` entries to keep in sync.
/// The SSR app shell must never come from a cache: a stale index.html
/// references the previous (content-hashed) asset bundle, and the old
/// assets are still in the SW cache — the user gets pinned to the old
/// build. HTML responses get `no-store`; the content-hashed asset
/// bundles keep their own caching.
pub async fn no_store_html_mw(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let resp = next.run(req).await;
    let is_html = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("text/html"))
        .unwrap_or(false);
    if is_html {
        let (mut parts, body) = resp.into_parts();
        parts.headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
        return axum::response::Response::from_parts(parts, body);
    }
    resp
}

pub fn install_pwa_routes(router: Router) -> Router {
    router
        .route("/sw.js", get(serve_sw_js))
        .route("/manifest.webmanifest", get(serve_manifest))
        .route("/icon-256.png", get(serve_icon_png))
}

// ---------------------------------------------------------------------------
// gzip middleware (v0.13.4 parity)
// ---------------------------------------------------------------------------

/// Compress compressible GET responses larger than 1 KB when the client
/// accepts gzip. Websocket upgrades (101), partial content (206) and
/// already-encoded responses pass through untouched.
pub async fn gzip_mw(req: Request, next: Next) -> Response {
    let is_get = req.method() == axum::http::Method::GET;
    let accept_gzip = req
        .headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("gzip"))
        .unwrap_or(false);
    let resp = next.run(req).await;
    if !is_get || !accept_gzip {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();
    if parts.status != StatusCode::OK {
        return Response::from_parts(parts, body);
    }
    let headers = &parts.headers;
    if headers.contains_key(header::CONTENT_ENCODING)
        || headers.contains_key(header::CONTENT_RANGE)
    {
        return Response::from_parts(parts, body);
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let compressible = ct.starts_with("text/html")
        || ct.starts_with("text/css")
        || ct.starts_with("text/plain")
        || ct.starts_with("text/markdown")
        || ct.starts_with("image/svg+xml")
        || ct.starts_with("application/javascript")
        || ct.starts_with("text/javascript")
        || ct.starts_with("application/json")
        || ct.starts_with("application/manifest+json");
    if !compressible {
        return Response::from_parts(parts, body);
    }
    let bytes = match to_bytes(body, 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    if bytes.len() <= 1024 {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    if enc.write_all(&bytes).is_err() {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let gz = match enc.finish() {
        Ok(gz) => gz,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };
    parts
        .headers
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    parts
        .headers
        .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Ok(v) = HeaderValue::from_str(&gz.len().to_string()) {
        parts.headers.insert(header::CONTENT_LENGTH, v);
    }
    Response::from_parts(parts, Body::from(gz))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid_json_with_both_purposes() {
        let v: serde_json::Value = serde_json::from_str(MANIFEST).expect("manifest json");
        assert_eq!(v["name"], "Halny Vibe Deck");
        assert_eq!(v["theme_color"], "#111115");
        let icons = v["icons"].as_array().expect("icons array");
        assert_eq!(icons.len(), 2);
        assert_eq!(icons[0]["purpose"], "any");
        assert_eq!(icons[1]["purpose"], "maskable");
    }

    #[test]
    fn sw_shell_is_network_first_and_skips_api() {
        assert!(SW_JS.contains("const CACHE = 'halny-vibe-deck-shell-__VERSION__'"));
        // network-first navigation
        assert!(SW_JS.contains("url.pathname === '/' || url.pathname === '/index.html'"));
        // api / login / sw self-fetch never cached
        assert!(SW_JS.contains("url.pathname.startsWith('/api/')"));
    }

    #[test]
    fn icon_png_encodes_from_ico() {
        let png = icon_png();
        assert!(!png.is_empty(), "icon png must encode from the repo ico");
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            "must start with the png magic"
        );
    }
}
