//! Stateless browser session tokens (stable v1.0.36 parity).
//!
//! The old store was an in-memory `HashMap<token, (user, expiry)>`, so every
//! process restart — a self-update, a systemd watchdog restart, a manual
//! `ctl restart` — logged every browser out. Tokens are now signed blobs:
//!
//! ```text
//! v1.<payload_b64url>.<mac_b64url>
//! payload = { "u": user, "iat": unix_secs, "exp": unix_secs, "v": <credentials version> }
//! mac     = HMAC-SHA256(key, "v1.<payload_b64url>")
//! ```
//!
//! * the key is persisted (`session-key` next to `config.json`, 0600) and is
//!   derived together with the current credential hash, so **changing the
//!   password invalidates every issued token** (`v` mismatch),
//! * `exp` is a sliding 180-day window capped at one year from `iat`
//!   (a token is re-issued by the login handler; no server state needed),
//! * an explicit logout puts the token's mac on an in-memory revoke list —
//!   the cookie is cleared in the browser at the same time, so a restart
//!   resurrecting a logged-out token has no practical effect, and stolen
//!   tokens stay dead for the lifetime of the process.
//!
//! No extra dependencies: HMAC-SHA256 is built from the `sha2` crate the
//! password hashing already uses.

use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Sliding validity window and the absolute cap (stable: 180 d / 1 year).
const WINDOW_SECS: u64 = 180 * 24 * 60 * 60;
const MAX_SECS: u64 = 365 * 24 * 60 * 60;

const VERSION_TAG: &str = "v1";

static REVOKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn b64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        out.push(T[(b[0] >> 2) as usize] as char);
        out.push(T[(((b[0] & 0b11) << 4) | (b[1] >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[(((b[1] & 0b1111) << 2) | (b[2] >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(T[(b[2] & 0b11_1111) as usize] as char);
        }
    }
    out
}

fn unb64url(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in bytes {
        acc = (acc << 6) | val(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let mut inner = Sha256::new();
    for b in &k {
        inner.update([b ^ 0x36]);
    }
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    for b in &k {
        outer.update([b ^ 0x5c]);
    }
    outer.update(inner);
    outer.finalize().to_vec()
}

/// Persistent signing key (`session-key` next to `config.json`), mixed with
/// the credential hash so a password change invalidates old tokens.
fn signing_key() -> Vec<u8> {
    let dir = dirs::config_dir().map(|d| d.join("halny-vibe-deck"));
    let key_path = dir.as_ref().map(|d| d.join("session-key"));
    let stored = key_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let secret = match stored {
        Some(s) => s,
        None => {
            let fresh = crate::config::random_hex(32);
            if let Some(path) = &key_path {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, &fresh);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }
            fresh
        }
    };
    // Bind the key to the current credentials: a password change rotates the
    // effective mac key, so every previously issued token stops validating.
    let cred = crate::config::EngineConfig::load()
        .auth
        .map(|a| format!("{}{}", a.salt, a.password_hash))
        .unwrap_or_default();
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(cred.as_bytes());
    h.finalize().to_vec()
}

/// Short, stable fingerprint of the credentials (the token's `v` field).
fn cred_version() -> String {
    let cred = crate::config::EngineConfig::load()
        .auth
        .map(|a| format!("{}{}", a.salt, a.password_hash))
        .unwrap_or_default();
    let digest = Sha256::digest(cred.as_bytes());
    hex_prefix(&digest, 8)
}

fn hex_prefix(bytes: &[u8], n: usize) -> String {
    bytes
        .iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Issue a fresh token for `username`.
pub fn issue(username: &str) -> String {
    let iat = now_secs();
    // A per-token nonce keeps two tokens issued in the same second distinct
    // (otherwise a revoke in one place would invalidate the other).
    let payload = format!(
        r#"{{"u":"{}","iat":{},"exp":{},"v":"{}","j":"{}"}}"#,
        username.replace('"', ""),
        iat,
        iat + WINDOW_SECS,
        cred_version(),
        crate::config::random_hex(8)
    );
    let body = format!("{VERSION_TAG}.{}", b64url(payload.as_bytes()));
    let mac = b64url(&hmac_sha256(&signing_key(), body.as_bytes()));
    format!("{body}.{mac}")
}

/// Validate a token, returning the username and whether it should be
/// re-issued (the sliding window has less than half of it left).
pub fn verify(token: &str) -> Option<(String, bool)> {
    let mut parts = token.split('.');
    let tag = parts.next()?;
    let payload_b64 = parts.next()?;
    let mac_b64 = parts.next()?;
    if parts.next().is_some() || tag != VERSION_TAG {
        return None;
    }
    if REVOKED
        .lock()
        .map(|r| r.iter().any(|m| m == mac_b64))
        .unwrap_or(false)
    {
        return None;
    }
    let body = format!("{tag}.{payload_b64}");
    let expected = b64url(&hmac_sha256(&signing_key(), body.as_bytes()));
    if !crate::config::constant_time_eq(expected.as_bytes(), mac_b64.as_bytes()) {
        return None;
    }
    let raw = String::from_utf8(unb64url(payload_b64)?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let user = v.get("u")?.as_str()?.to_string();
    let iat = v.get("iat")?.as_u64()?;
    let exp = v.get("exp")?.as_u64()?;
    let ver = v.get("v")?.as_str()?;
    if ver != cred_version() {
        return None; // password changed since the token was issued
    }
    let now = now_secs();
    if now >= exp || now >= iat + MAX_SECS {
        return None;
    }
    let refresh = exp.saturating_sub(now) < WINDOW_SECS / 2;
    Some((user, refresh))
}

/// Explicit logout: the token stops validating for the rest of the process.
pub fn revoke(token: &str) {
    if let Some(mac) = token.split('.').nth(2) {
        if let Ok(mut list) = REVOKED.lock() {
            if !list.iter().any(|m| m == mac) {
                list.push(mac.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let token = issue("e2e");
        assert!(token.starts_with("v1."));
        let (user, _refresh) = verify(&token).expect("valid token");
        assert_eq!(user, "e2e");

        // Any mutation of the payload invalidates the mac.
        let mut tampered = token.clone();
        let idx = 4;
        let replacement = if tampered.as_bytes()[idx] == b'A' { 'B' } else { 'A' };
        tampered.replace_range(idx..idx + 1, &replacement.to_string());
        assert!(verify(&tampered).is_none());
    }

    #[test]
    fn revoked_token_is_rejected() {
        let token = issue("e2e");
        assert!(verify(&token).is_some());
        revoke(&token);
        assert!(verify(&token).is_none());
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        for bad in ["", "v1", "v1.only", "v2.a.b", "v1.a.b.c", "v1.!!!.???"] {
            assert!(verify(bad).is_none(), "{bad:?} must not validate");
        }
    }
}
