//! Browser sign-in flow (PKCE) — server side, TUI parity
//! (`cli-rust/src/setup/auth/sign_in_flow.rs:30`).
//!
//! The client (wasm/browser) has no network access, so the engine
//! performs the HTTP calls with `curl` (same pattern as
//! `remote.rs::http_get_json`, extended with POST + JSON body):
//!
//! 1. `code_verifier` (64 B from the OS CSPRNG), `code_challenge` =
//!    BASE64URL-NO-PAD(SHA256(verifier)),
//! 2. `POST {browserAuthApiBaseUrl}/vibe/sign-in`
//!    `{"code_challenge", "code_challenge_method":"S256"}` →
//!    `{process_id, sign_in_url, poll_url, expires_at}`,
//! 3. poll `poll_url` (client calls every 3 s): `pending | completed
//!    (→ exchange) | expired | denied | error` (3 consecutive
//!    network errors = failure),
//! 4. exchange `(process_id, exchange_token, code_verifier)` →
//!    `{api_key}` → `setup/store-credential` + `setup/submit-choices`
//!    travel through the transport (engine `login_browser_poll`).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// State of one browser sign-in flow; lives in
/// `Engine: Mutex<Option<..>>` so the verifier survives between
/// server-fn calls. Never sent to the UI (verifier stays here).
pub struct BrowserSignInFlow {
    pub process_id: String,
    pub sign_in_url: String,
    pub poll_url: String,
    pub expires_at_ms: i64,
    pub verifier: String,
    /// Browser-auth API base the flow was started against (exchange
    /// endpoint derivation) — from `setup/status.provider`.
    pub api_base: String,
    /// Consecutive network errors (3 in a row = failure).
    pub net_errors: u32,
}

/// Result of one poll step (the client calls every 3 s).
pub enum PollOutcome {
    Pending,
    /// Exchange already performed inside; the api key is ready to
    /// store (`setup/store-credential` by the engine).
    Completed { api_key: String },
    Expired,
    Denied,
    Failed(String),
}

/// Start: verifier → challenge=S256 → POST {base}/vibe/sign-in.
pub fn browser_sign_in_begin(browser_auth_api_base_url: &str) -> Result<BrowserSignInFlow, String> {
    let verifier = gen_verifier();
    let challenge = base64url_no_pad(&Sha256::digest(verifier.as_bytes()));
    let body = json!({
        "code_challenge": challenge,
        "code_challenge_method": "S256",
    });
    let res = http_post_json(
        &format!("{browser_auth_api_base_url}/vibe/sign-in"),
        &body,
        30,
    )?;
    let v: Value = serde_json::from_str(&res).map_err(|e| format!("sign-in: bad JSON: {e}"))?;
    let s = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    let process_id = s(&["process_id", "processId"]);
    let sign_in_url = s(&["sign_in_url", "signInUrl"]);
    let poll_url = s(&["poll_url", "pollUrl"]);
    if process_id.is_empty() || sign_in_url.is_empty() || poll_url.is_empty() {
        return Err(format!("sign-in: incomplete response: {res:.200}"));
    }
    // epoch ms (int) or ISO-ish string on the wire — tolerate both.
    let expires_at_ms = v
        .get("expires_at_ms")
        .or_else(|| v.get("expiresAtMs"))
        .and_then(Value::as_i64)
        .or_else(|| {
            v.get("expires_at")
                .or_else(|| v.get("expiresAt"))
                .and_then(Value::as_i64)
        })
        .unwrap_or_else(|| now_ms() + 15 * 60 * 1000);
    Ok(BrowserSignInFlow {
        process_id,
        sign_in_url,
        poll_url,
        expires_at_ms,
        verifier,
        api_base: browser_auth_api_base_url.to_string(),
        net_errors: 0,
    })
}

/// One poll step: GET poll_url; "completed" → immediate exchange.
pub fn browser_sign_in_poll(flow: &mut BrowserSignInFlow) -> PollOutcome {
    if now_ms() >= flow.expires_at_ms {
        return PollOutcome::Expired;
    }
    let body = match http_get_json(&flow.poll_url, 15) {
        Ok(b) => b,
        Err(e) => {
            flow.net_errors += 1;
            if flow.net_errors >= 3 {
                return PollOutcome::Failed(format!("poll: {e} (3 network errors)"));
            }
            return PollOutcome::Pending;
        }
    };
    flow.net_errors = 0;
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return PollOutcome::Failed(format!("poll: bad JSON: {body:.200}"));
    };
    // `{"status": "pending"}` — also accept a bare string body.
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| v.as_str())
        .unwrap_or_default()
        .to_string();
    match status.as_str() {
        "pending" => PollOutcome::Pending,
        "expired" => PollOutcome::Expired,
        "denied" => PollOutcome::Denied,
        "completed" => {
            let exchange_token = ["exchange_token", "exchangeToken"]
                .iter()
                .find_map(|k| v.get(*k).and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            if exchange_token.is_empty() {
                return PollOutcome::Failed("poll: completed without exchange_token".into());
            }
            exchange(flow, &exchange_token)
        }
        "error" => {
            let msg = v
                .get("message")
                .or_else(|| v.get("detail"))
                .and_then(Value::as_str)
                .unwrap_or("server error");
            PollOutcome::Failed(msg.to_string())
        }
        other => PollOutcome::Failed(format!("poll: unknown status {other:.60}")),
    }
}

/// Exchange `(process_id, exchange_token, code_verifier)` → `{api_key}`
/// (403 = workspace not eligible, 410 = expired, 9004 = no access —
/// all surface as `Failed` with the body/stderr text).
fn exchange(flow: &BrowserSignInFlow, exchange_token: &str) -> PollOutcome {
    let body = json!({
        "process_id": flow.process_id,
        "exchange_token": exchange_token,
        "code_verifier": flow.verifier,
    });
    match http_post_json(&format!("{}/vibe/sign-in/exchange", flow.api_base), &body, 30) {
        Ok(res) => {
            let api_key = serde_json::from_str::<Value>(&res)
                .ok()
                .and_then(|v| {
                    v.get("api_key")
                        .or_else(|| v.get("apiKey"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            if api_key.is_empty() {
                PollOutcome::Failed(format!("exchange: no api_key in {res:.200}"))
            } else {
                PollOutcome::Completed { api_key }
            }
        }
        Err(e) => PollOutcome::Failed(format!("exchange: {e}")),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 64 B from the OS CSPRNG (getrandom), hex-encoded — the TUI
/// verifier shape (`sign_in_flow.rs:137-150`).
fn gen_verifier() -> String {
    let mut bytes = [0u8; 64];
    getrandom::getrandom(&mut bytes).expect("OS CSPRNG unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// BASE64URL without padding (RFC 4648 §5) — ~15 lines instead of a
/// new dependency (`base64` is not in the engine graph).
fn base64url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// HTTP GET via curl (same pattern as `remote.rs::http_get_json`).
fn http_get_json(url: &str, timeout_s: u64) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", &timeout_s.to_string(), url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// HTTP POST with a JSON body via curl (`-X POST -H Content-Type -d`).
fn http_post_json(url: &str, body: &Value, timeout_s: u64) -> Result<String, String> {
    let payload = body.to_string();
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            &timeout_s.to_string(),
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &payload,
            url,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        // curl -f puts the HTTP error code on stderr; surface the
        // body too when present (403/410/9004 carry a JSON detail).
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Err(if stdout.is_empty() {
            stderr
        } else {
            format!("{stdout} ({stderr})")
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_rfc_vectors() {
        assert_eq!(base64url_no_pad(b""), "");
        assert_eq!(base64url_no_pad(b"f"), "Zg");
        assert_eq!(base64url_no_pad(b"fo"), "Zm8");
        assert_eq!(base64url_no_pad(b"foo"), "Zm9v");
        assert_eq!(base64url_no_pad(b"foob"), "Zm9vYg");
        assert_eq!(base64url_no_pad(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_no_pad(b"foobar"), "Zm9vYmFy");
        // RFC 4648 §5 test vectors (URL-safe alphabet, no padding).
        assert_eq!(base64url_no_pad(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url_no_pad(&[0xff]), "_w");
    }

    #[test]
    fn verifier_is_hex_and_random() {
        let v = gen_verifier();
        assert_eq!(v.len(), 128); // 64 B → hex
        assert!(v.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(v, gen_verifier());
    }

    /// TUI parity (`sign_in_flow.rs:137-150`): challenge =
    /// BASE64URL(SHA256(verifier)). Verified against the known
    /// PKCE test vector verifier "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    /// (RFC 7636 appendix B) whose S256 challenge is "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".
    #[test]
    fn challenge_matches_pkce_rfc7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64url_no_pad(&Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn poll_maps_statuses() {
        // expired by wall clock, before any HTTP
        let mut flow = BrowserSignInFlow {
            process_id: "p".into(),
            sign_in_url: "u".into(),
            poll_url: "http://127.0.0.1:1/poll".into(),
            expires_at_ms: now_ms() - 1,
            verifier: "v".into(),
            api_base: "http://127.0.0.1:1".into(),
            net_errors: 0,
        };
        assert!(matches!(
            browser_sign_in_poll(&mut flow),
            PollOutcome::Expired
        ));
    }

    #[test]
    fn three_network_errors_fail() {
        // port 1 refuses instantly — three polls must exhaust retries
        let mut flow = BrowserSignInFlow {
            process_id: "p".into(),
            sign_in_url: "u".into(),
            poll_url: "http://127.0.0.1:1/poll".into(),
            expires_at_ms: now_ms() + 60_000,
            verifier: "v".into(),
            api_base: "http://127.0.0.1:1".into(),
            net_errors: 0,
        };
        assert!(matches!(
            browser_sign_in_poll(&mut flow),
            PollOutcome::Pending
        ));
        assert!(matches!(
            browser_sign_in_poll(&mut flow),
            PollOutcome::Pending
        ));
        assert!(matches!(
            browser_sign_in_poll(&mut flow),
            PollOutcome::Failed(_)
        ));
    }
}
