//! Server functions for the first-run SetupView: detect `uv` + `vibe-acp`,
//! install mistral-vibe via `uv tool install` on a background thread with a
//! pollable progress log. Adapted from the Nexus main-process setup module
//! (same steps: uv → mistral-vibe → locate vibe-acp → save config).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Detected binaries (Nexus SetupStatus).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupStatus {
    pub vib_acp_found: bool,
    pub vib_acp_path: Option<String>,
    pub uv_found: bool,
    pub uv_path: Option<String>,
}

/// One progress line of the install log (kind: info | success | error).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupLine {
    pub message: String,
    pub kind: String,
}

#[cfg(feature = "server")]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    pub struct SetupState {
        pub lines: Vec<SetupLine>,
        pub running: bool,
        /// Some(true) = finished OK, Some(false) = finished with error.
        pub finished: Option<bool>,
    }

    pub static SETUP: Mutex<Option<SetupState>> = Mutex::new(None);
    static STARTED: AtomicBool = AtomicBool::new(false);

    pub fn push(message: impl Into<String>, kind: &str) {
        let mut g = SETUP.lock().unwrap();
        g.get_or_insert_with(|| SetupState {
            lines: Vec::new(),
            running: true,
            finished: None,
        })
        .lines
        .push(SetupLine {
            message: message.into(),
            kind: kind.to_string(),
        });
    }

    /// `which NAME` — resolve through PATH.
    fn which(name: &str) -> Option<String> {
        let out = std::process::Command::new("which")
            .arg(name)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let first = s.split('\n').next().unwrap_or("").trim().to_string();
        if first.is_empty() {
            None
        } else {
            Some(first)
        }
    }

    /// `which` first, then the known uv install locations (Nexus
    /// candidatePaths, unix branch).
    pub fn find_binary(name: &str) -> Option<String> {
        if let Some(p) = which(name) {
            return Some(p);
        }
        let home = std::env::var("HOME").ok()?;
        let home = std::path::PathBuf::from(home);
        let candidates = [
            home.join(".local/share/uv/tools/mistral-vibe/bin")
                .join(name),
            home.join(".local/bin").join(name),
            std::path::PathBuf::from("/usr/local/bin").join(name),
        ];
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .map(|p| p.display().to_string())
    }

    pub fn check() -> SetupStatus {
        let vib = find_binary("vibe-acp");
        let uv = find_binary("uv");
        SetupStatus {
            vib_acp_found: vib.is_some(),
            vib_acp_path: vib,
            uv_found: uv.is_some(),
            uv_path: uv,
        }
    }

    fn run_cmd(cmd: &str, args: &[&str]) -> Result<(), String> {
        let out = std::process::Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| format!("{cmd}: {e}"))?;
        for l in String::from_utf8_lossy(&out.stdout).lines() {
            if !l.trim().is_empty() {
                push(l, "info");
            }
        }
        for l in String::from_utf8_lossy(&out.stderr).lines() {
            if !l.trim().is_empty() {
                push(l, "info");
            }
        }
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("exit code {}", out.status.code().unwrap_or(-1)))
        }
    }

    fn real_install() -> bool {
        let st = check();
        // Step 1: uv
        let uv_path = match st.uv_path {
            Some(p) => {
                push(format!("uv: {p}"), "success");
                p
            }
            None => {
                push("Instalowanie uv package manager…", "info");
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("curl -LsSf https://astral.sh/uv/install.sh | sh")
                    .output();
                match out {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        let tail = String::from_utf8_lossy(&o.stderr);
                        push(format!("uv install failed: {tail}"), "error");
                        return false;
                    }
                    Err(e) => {
                        push(format!("uv install failed: {e}"), "error");
                        return false;
                    }
                }
                match find_binary("uv") {
                    Some(p) => {
                        push(format!("uv zainstalowano: {p}"), "success");
                        p
                    }
                    None => {
                        push("uv nie znaleziono po instalacji.", "error");
                        return false;
                    }
                }
            }
        };
        // Step 2: mistral-vibe
        push("Instalowanie mistral-vibe…", "info");
        if let Err(e) = run_cmd(&uv_path, &["tool", "install", "mistral-vibe"]) {
            push(format!("Błąd instalacji: {e}"), "error");
            return false;
        }
        push("mistral-vibe zainstalowano.", "success");
        // Step 3: locate vibe-acp + persist the path
        match find_binary("vibe-acp") {
            Some(p) => {
                push(format!("vibe-acp gotowy: {p}"), "success");
                vibedeck_engine::Engine::global().update_config(|c| c.acp_path = p.clone());
                true
            }
            None => {
                push("vibe-acp nie znaleziono po instalacji.", "error");
                false
            }
        }
    }

    /// Start the install on a background thread (idempotent while running).
    pub fn install() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            {
                let mut g = SETUP.lock().unwrap();
                *g = Some(SetupState {
                    lines: Vec::new(),
                    running: true,
                    finished: None,
                });
            }
            let ok = real_install();
            {
                let mut g = SETUP.lock().unwrap();
                if let Some(st) = g.as_mut() {
                    st.running = false;
                    st.finished = Some(ok);
                }
            }
            STARTED.store(false, Ordering::SeqCst);
        });
    }

    pub fn log() -> (Vec<SetupLine>, bool, Option<bool>) {
        let g = SETUP.lock().unwrap();
        match g.as_ref() {
            Some(st) => (st.lines.clone(), st.running, st.finished),
            None => (Vec::new(), false, None),
        }
    }
}

#[cfg(feature = "server")]
fn check(_token: &str) -> Result<(), ServerFnError> {
    // No token enforcement (see api.rs): requests arrive from the trusted
    // local listener or through the authenticated remote HTTPS listener.
    Ok(())
}

/// Local-only guard (Nexus parity: setup:install is a web-blocked
/// channel) — installing the CLI is only allowed from the local interface.
#[cfg(feature = "server")]
fn require_local() -> Result<(), ServerFnError> {
    if let Some(ctx) = dioxus_fullstack::FullstackContext::current() {
        let headers = ctx.parts_mut().headers.clone();
        let origin = headers
            .get("x-vd-origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if origin == "remote" {
            return Err(ServerFnError::new(
                "the CLI can only be installed from the local interface",
            ));
        }
    }
    Ok(())
}

/// Detect uv + vibe-acp (safe to call without a token: it only inspects
/// local binaries, it is what decides whether SetupView shows).
#[server]
pub async fn api_setup_status() -> Result<SetupStatus, ServerFnError> {
    Ok(imp::check())
}

/// Kick off the install in the background (poll api_setup_log for progress).
#[server]
pub async fn api_setup_install(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    require_local()?;
    imp::install();
    Ok(())
}

/// Current install log: (lines, still-running, finished-ok).
#[server]
pub async fn api_setup_log(
    token: String,
) -> Result<(Vec<SetupLine>, bool, Option<bool>), ServerFnError> {
    check(&token)?;
    Ok(imp::log())
}
