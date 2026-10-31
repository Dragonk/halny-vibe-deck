//! Server functions for the first-run SetupView: detect `uv`,
//! `mistral-vibe` and `vibe-app-server`, install the CLI through
//! `uv tool install` on an engine background thread with a pollable
//! progress log. Adapted from the Nexus main-process setup module
//! (same steps: uv → mistral-vibe → locate vibe-app-server).

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Detected binaries (Nexus SetupStatus).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupStatus {
    /// The `vibe-app-server` agent binary (the only transport).
    pub agent_found: bool,
    pub agent_path: Option<String>,
    /// The `vibe` CLI (installed together with the app-server).
    pub cli_found: bool,
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

    pub fn check() -> SetupStatus {
        let st = halny_vibe_deck_setup::check_setup();
        // Explicit config override wins over auto-detection.
        let agent = {
            let cfg = halny_vibe_deck_engine::Engine::global().config();
            if !cfg.agent_path.is_empty() {
                Some(std::path::PathBuf::from(cfg.agent_path))
            } else {
                st.appserver_path.clone()
            }
        };
        SetupStatus {
            agent_found: agent.is_some(),
            agent_path: agent.map(|p| p.display().to_string()),
            cli_found: st.vibe_cli_version.is_some(),
            uv_found: st.uv_path.is_some(),
            uv_path: st.uv_path.map(|p| p.display().to_string()),
        }
    }

    pub fn log() -> (Vec<SetupLine>, bool, Option<bool>) {
        let engine = halny_vibe_deck_engine::Engine::global();
        let g = engine.setup_install_state();
        (
            g.lines
                .into_iter()
                .map(|l| SetupLine {
                    message: l.message,
                    kind: l.kind,
                })
                .collect(),
            g.running,
            g.finished,
        )
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

/// Detect uv + the vibe-app-server agent (safe to call without a token:
/// it only inspects local binaries, it is what decides whether SetupView
/// shows).
#[server]
pub async fn api_setup_status() -> Result<SetupStatus, ServerFnError> {
    Ok(imp::check())
}

/// Kick off the install in the background (poll api_setup_log for progress).
#[server]
pub async fn api_setup_install(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    require_local()?;
    halny_vibe_deck_engine::Engine::global().setup_install();
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
