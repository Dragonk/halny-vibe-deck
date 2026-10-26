//! Engine configuration (~/.config/mistraldeck/config.json) + auth entries.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

/// Credentials for browser access (headless / remote-control mode).
/// sha256(salt + password) - personal/LAN-grade protection, not a
/// public-internet-grade KDF. Keep the port behind a firewall anyway.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthEntry {
    pub username: String,
    pub password_hash: String,
    pub salt: String,
}

impl AuthEntry {
    pub fn new(username: &str, password: &str) -> Self {
        let salt = random_hex(16);
        let password_hash = hash_password(&salt, password);
        Self {
            username: username.to_string(),
            password_hash,
            salt,
        }
    }

    pub fn verify(&self, username: &str, password: &str) -> bool {
        let given = hash_password(&self.salt, password);
        let ok_user = constant_time_eq(self.username.as_bytes(), username.as_bytes());
        let ok_pass = constant_time_eq(self.password_hash.as_bytes(), given.as_bytes());
        ok_user && ok_pass
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn hash_password(salt: &str, password: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(password.as_bytes());
    hex::encode(h.finalize())
}

/// Random hex token from the OS entropy pool (/dev/urandom on Unix).
pub fn random_hex(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf).map(|_| ()))
        .is_ok();
    if !filled {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut seed = now as u64 ^ (buf.as_ptr() as u64);
        for b in buf.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *b = (seed >> 33) as u8;
        }
    }
    hex::encode(buf)
}

/// The subset of ~/.config/mistraldeck/config.json the engine cares about.
/// Unknown fields (egui-era settings) are preserved on save via a raw
/// JSON round-trip.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub acp_path: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub preferred_model: Option<String>,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub auth: Option<AuthEntry>,
    #[serde(default)]
    pub remote_enabled: bool,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    #[serde(default)]
    pub dock_open: bool,
    #[serde(default)]
    pub sidebar_sessions_limit: Option<u32>,
    #[serde(default)]
    pub auto_close_idle_minutes: Option<u32>,
    #[serde(default)]
    pub recent_folders: Vec<String>,
    #[serde(default)]
    pub hidden_sessions: Vec<String>,
    #[serde(default)]
    pub hidden_projects: Vec<String>,
    /// SSH remotes where vibe-acp runs (Nexus remote connections).
    #[serde(default)]
    pub remote_connections: Vec<mistraldeck_core::snapshot::RemoteConnection>,
}

fn default_remote_port() -> u16 {
    8080
}

impl EngineConfig {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("mistraldeck").join("config.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).ok().unwrap_or_default(),
            Err(_) => {
                // Migration from the pre-rebrand location
                // (~/.config/vibedeck/config.json): read it once; saves
                // always go to the new mistraldeck path.
                let old = dirs::config_dir().map(|d| d.join("vibedeck").join("config.json"));
                old.and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default()
            }
        }
    }

    /// Save, preserving any extra fields other tools wrote to the file.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let mut root: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if let Ok(me) = serde_json::to_value(self) {
            if let Some(map) = me.as_object() {
                for (k, v) in map {
                    root.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap_or_default());
    }

    /// Set (or replace) the login credentials from the CLI.
    pub fn set_login(username: &str, password: &str) {
        let mut cfg = Self::load();
        cfg.auth = Some(AuthEntry::new(username, password));
        cfg.save();
    }
}
