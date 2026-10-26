//! Server functions: the only bridge between the UI client and the engine.
//!
//! Auth model (Nexus parity): the local server binds 127.0.0.1 and is
//! trusted without a login; remote browser access goes through the HTTPS
//! listener (serve.rs) whose middleware authenticates every request with a
//! session cookie before it can reach any of these functions. The token
//! parameters are kept for protocol compatibility and are not enforced.

use dioxus::prelude::*;
use vibedeck_core::snapshot::Snapshot;

#[cfg(feature = "server")]
fn check(_token: &str) -> Result<(), ServerFnError> {
    // No token enforcement: requests arrive either from the local
    // 127.0.0.1 listener (trusted) or through the remote HTTPS listener,
    // whose middleware already validated the session cookie.
    Ok(())
}

/// Local-only guard: reject requests that were routed through the remote
/// HTTPS listener (marked `x-vd-origin: remote` by its middleware). Used
/// for credential editing — login/password/port can only be changed from
/// the local interface.
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
                "credentials can only be edited from the local interface",
            ));
        }
    }
    Ok(())
}

#[server]
pub async fn api_snapshot(token: String) -> Result<Snapshot, ServerFnError> {
    check(&token)?;
    Ok(vibedeck_engine::Engine::global().snapshot())
}

#[server]
pub async fn api_create_session(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().create_local_session();
    Ok(())
}

#[server]
pub async fn api_switch_session(token: String, index: usize) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().switch_session(index);
    Ok(())
}

#[server]
pub async fn api_open_backend_session(token: String, session_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().open_backend_session(&session_id);
    Ok(())
}

/// Send a prompt. `@path` markers in the text are resolved server-side
/// (Nexus parity): every readable file/directory under the working
/// directory is inlined as a `<vibe_attach_*>` block and recorded on the
/// user message so the UI can render attachment chips. Unresolvable
/// markers stay as typed.
#[server]
pub async fn api_send_message(token: String, text: String) -> Result<(), ServerFnError> {
    check(&token)?;
    let (expanded, attachments) = resolve_attachments(&text);
    vibedeck_engine::Engine::global().send_message_with(expanded, attachments);
    Ok(())
}

#[server]
pub async fn api_cancel(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().cancel();
    Ok(())
}

#[server]
pub async fn api_approve(token: String, request_id: String, option_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().approve(request_id, option_id);
    Ok(())
}

#[server]
pub async fn api_answer_question(token: String, request_id: String, answer: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().answer_question(request_id, answer);
    Ok(())
}

#[server]
pub async fn api_set_config_option(token: String, key: String, value: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().set_config_option(&key, value);
    Ok(())
}

#[server]
pub async fn api_refresh_sessions(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().refresh_sessions();
    Ok(())
}

/// Persist UI preferences: language, theme, dock open, sidebar limit.
#[server]
pub async fn api_set_prefs(
    token: String,
    language: String,
    theme: String,
    dock_open: bool,
    sidebar_sessions_limit: u32,
) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().set_prefs(language, theme, dock_open, sidebar_sessions_limit);
    Ok(())
}

/// Configure remote-access credentials (login + password + port + enabled).
/// The password is hashed (sha256 with salt) server-side; nothing is
/// ever displayed back to the client. LOCAL-ONLY: requests routed through
/// the remote HTTPS listener are rejected (credentials must be edited
/// from the local interface).
#[server]
pub async fn api_set_remote(
    token: String,
    username: String,
    password: String,
    port: u16,
    enable: bool,
) -> Result<(), ServerFnError> {
    check(&token)?;
    require_local()?;
    if username.trim().is_empty() || password.is_empty() {
        return Err(ServerFnError::new("username and password required"));
    }
    vibedeck_engine::Engine::global()
        .set_remote(username.trim(), &password, port, enable);
    Ok(())
}

/// Hide a session from the sidebar (nothing is deleted from disk).
#[server]
pub async fn api_hide_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().hide_session(&key);
    Ok(())
}

#[server]
pub async fn api_hide_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().hide_project(&wd);
    Ok(())
}

#[server]
pub async fn api_restore_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().restore_session(&key);
    Ok(())
}

#[server]
pub async fn api_restore_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().restore_project(&wd);
    Ok(())
}

// ---------------------------------------------------------------------------
// @-attachment resolution (server-side, Nexus parity)
// ---------------------------------------------------------------------------

/// Expand `@path` markers in a prompt into inline `<vibe_attach_*>`
/// blocks (like the Nexus InputBar) and collect display attachments.
/// Only paths inside the current working directory are readable; the
/// content is capped so a huge file cannot blow up the prompt.
#[cfg(feature = "server")]
fn resolve_attachments(text: &str) -> (String, Vec<vibedeck_core::types::Attachment>) {
    const MAX_FILE_BYTES: usize = 256 * 1024;
    const MAX_DIR_ENTRIES: usize = 400;

    let engine = vibedeck_engine::Engine::global();
    let cwd = engine.workdir();
    let mut expanded = text.to_string();
    let mut attachments: Vec<vibedeck_core::types::Attachment> = Vec::new();

    if cwd.is_empty() || !text.contains('@') {
        return (expanded, attachments);
    }

    // Tokens starting with `@` (Nexus regex: /@([^\s]+)/g) — collect unique
    // markers first so the same path referenced twice is only read once.
    let mut markers: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        if let Some(raw) = tok.strip_prefix('@') {
            if !raw.is_empty() && !markers.iter().any(|m| m == tok) {
                markers.push(tok.to_string());
            }
        }
    }

    let root = std::path::PathBuf::from(&cwd);
    for marker in markers {
        let raw = &marker[1..]; // strip '@'
        let is_dir = raw.ends_with('/');
        let rel = raw.trim_end_matches('/');
        if rel.is_empty() || rel.contains("..") || rel.starts_with('/') {
            continue; // no traversal outside the working directory
        }
        let full = root.join(rel);
        let (block, content) = if is_dir {
            let listing = match std::fs::read_dir(&full) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .take(MAX_DIR_ENTRIES)
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            if e.path().is_dir() {
                                format!("{}/", name)
                            } else {
                                name
                            }
                        })
                        .collect();
                    names.sort();
                    names.join("\n")
                }
                Err(_) => continue, // not a readable directory — leave marker as typed
            };
            let name = full
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rel.to_string());
            (
                format!(
                    "<vibe_attach_directory><name>{name}</name><location>{raw}</location><content>{listing}</content></vibe_attach_directory>"
                ),
                listing,
            )
        } else {
            let bytes = match std::fs::metadata(&full).map(|m| m.len()) {
                Ok(len) if len <= MAX_FILE_BYTES as u64 => std::fs::read_to_string(&full),
                _ => continue, // missing or too large — leave marker as typed
            };
            let content = match bytes {
                Ok(c) => c,
                Err(_) => continue, // binary / unreadable — leave marker as typed
            };
            let name = full
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rel.to_string());
            (
                format!(
                    "<vibe_attach_file><name>{name}</name><location>{raw}</location><content>{content}</content></vibe_attach_file>"
                ),
                content,
            )
        };
        attachments.push(vibedeck_core::types::Attachment {
            path: raw.to_string(),
            is_dir,
            content,
        });
        // Replace only the first occurrence of this marker (Nexus does the
        // same via String.replace).
        if let Some(pos) = expanded.find(marker.as_str()) {
            expanded.replace_range(pos..pos + marker.len(), &block);
        }
    }

    (expanded, attachments)
}

// ---------------------------------------------------------------------------
// SSH remote connections (Nexus parity)
// ---------------------------------------------------------------------------

/// Saved SSH remote connections (Settings editor + sidebar groups).
#[server]
pub async fn api_remote_connections(
    token: String,
) -> Result<Vec<vibedeck_core::snapshot::RemoteConnection>, ServerFnError> {
    check(&token)?;
    Ok(vibedeck_engine::Engine::global().config().remote_connections)
}

/// Persist the full remote-connection list. Allowed through the remote
/// HTTPS listener too (Nexus only protects the serve/credentials section).
#[server]
pub async fn api_save_remote_connections(
    token: String,
    conns: Vec<vibedeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    check(&token)?;
    for c in &conns {
        if c.name.trim().is_empty() || c.ssh_cmd.trim().is_empty() {
            return Err(ServerFnError::new("name and ssh command required"));
        }
    }
    vibedeck_engine::Engine::global().set_remote_connections(conns);
    Ok(())
}

/// Test an (unsaved) connection draft: reachability + vibe-acp presence.
#[server]
pub async fn api_test_remote(
    token: String,
    conn: vibedeck_core::snapshot::RemoteConnection,
) -> Result<vibedeck_core::snapshot::RemoteTestResult, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || vibedeck_engine::remote::test_remote(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Install (or upgrade) vibe-acp on the remote host.
/// LOCAL-ONLY (Nexus WEB_BLOCKED has remote:install-acp).
#[server]
pub async fn api_install_remote_acp(
    token: String,
    conn: vibedeck_core::snapshot::RemoteConnection,
) -> Result<vibedeck_core::snapshot::RemoteInstallResult, ServerFnError> {
    check(&token)?;
    require_local()?;
    tokio::task::spawn_blocking(move || vibedeck_engine::remote::install_remote_acp(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// CLI sessions discovered on a remote host (JSON-RPC over ssh).
#[server]
pub async fn api_remote_sessions(
    token: String,
    conn_id: String,
) -> Result<Vec<vibedeck_core::snapshot::RemoteSessionInfo>, ServerFnError> {
    check(&token)?;
    let Some(conn) = vibedeck_engine::Engine::global().remote_connection(&conn_id) else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || vibedeck_engine::remote::list_remote_sessions(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Quick liveness check for the sidebar status dot.
#[server]
pub async fn api_check_remote_status(
    token: String,
    conn_id: String,
) -> Result<vibedeck_core::snapshot::RemoteStatus, ServerFnError> {
    check(&token)?;
    let Some(conn) = vibedeck_engine::Engine::global().remote_connection(&conn_id) else {
        return Ok(vibedeck_core::snapshot::RemoteStatus::default());
    };
    tokio::task::spawn_blocking(move || vibedeck_engine::remote::check_remote_status(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// New session on an SSH remote (sidebar "+" per location).
#[server]
pub async fn api_start_remote_session(
    token: String,
    conn_id: String,
    location: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().create_remote_session(&conn_id, &location);
    Ok(())
}

/// Open a CLI session discovered on an SSH remote (replays history).
#[server]
pub async fn api_open_remote_session(
    token: String,
    conn_id: String,
    session_id: String,
    title: String,
    cwd: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().open_remote_session(&conn_id, &session_id, &title, &cwd);
    Ok(())
}

/// A project agent (`<cwd>/.vibe/agents/<name>.toml`) surfaced in the
/// mode chip dropdown — Nexus `vibe:get-agents("project")` view.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentChoiceView {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

/// Project-level agents for the mode chip (top-level only, no subagents).
#[server]
pub async fn api_project_agents(
    token: String,
    cwd: String,
) -> Result<Vec<AgentChoiceView>, ServerFnError> {
    check(&token)?;
    let dir = std::path::Path::new(&cwd).join(".vibe").join("agents");
    let agents = vibedeck_vibe::AgentFile::load_all_in(&dir);
    Ok(agents
        .into_iter()
        .filter(|a| a.agent_type.as_deref() != Some("subagent") && !a.name.is_empty())
        .map(|a| AgentChoiceView {
            name: a.name,
            display_name: a.display_name,
            description: a.description,
        })
        .collect())
}

/// Copy a project agent into `~/.vibe/agents/` so the CLI backend can
/// activate it as a mode (Nexus `vibe:expose-project-agent`).
#[server]
pub async fn api_expose_project_agent(
    token: String,
    cwd: String,
    name: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    // Sanitize: the name selects a file directly under the agents dir.
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name == "."
    {
        return Err(ServerFnError::new("invalid agent name"));
    }
    let src = std::path::Path::new(&cwd)
        .join(".vibe")
        .join("agents")
        .join(format!("{name}.toml"));
    if !src.is_file() {
        return Err(ServerFnError::new("project agent file not found"));
    }
    let dest_dir = vibedeck_vibe::vibe_home().join("agents");
    std::fs::create_dir_all(&dest_dir).map_err(|e| ServerFnError::new(e.to_string()))?;
    let dest = dest_dir.join(format!("{name}.toml"));
    std::fs::copy(&src, &dest).map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}
