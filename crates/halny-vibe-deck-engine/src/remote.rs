//! SSH remote connections (Nexus parity): run commands over the user's
//! `ssh` command, test/install the CLI on remote hosts and list remote
//! CLI sessions by speaking the same JSON-RPC protocol vibe-app-server
//! uses locally - only over the ssh pipe.
//!
//! All functions are blocking (they spawn subprocesses); call them from
//! `tokio::task::spawn_blocking` in server contexts.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use halny_vibe_deck_core::snapshot::{CliUpdateInfo, CliUpgradeResult, RemoteConnection, RemoteInstallResult, RemoteSessionInfo, RemoteStatus, RemoteTestResult};

/// Split an ssh command string into argv tokens (respects quotes) -
/// port of Nexus `tokenizeCmd`.
pub fn tokenize_cmd(cmd: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in cmd.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Quote a string for the remote POSIX shell (single quotes, escape).
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Insert OpenSSH connect options into a tokenized ssh command.
/// Round 5 (R1): EVERY ssh spawn must carry these — without
/// `StrictHostKeyChecking=accept-new` a first connect hangs on the
/// interactive host-key prompt (the sidebar's remote session list then
/// silently comes back empty) and without `ConnectTimeout` a dead host
/// blocks until the caller's deadline. Only for an OpenSSH binary and
/// only when the user's own command passes no `-o` options.
pub fn ssh_connect_opts(tokens: &[String]) -> Vec<String> {
    let Some(ssh_idx) = tokens.iter().position(|t| {
        std::path::Path::new(t)
            .file_name()
            .and_then(|n| n.to_str())
            == Some("ssh")
    }) else {
        return tokens.to_vec();
    };
    if tokens.iter().any(|t| t == "-o") {
        return tokens.to_vec();
    }
    let mut out: Vec<String> = tokens[..=ssh_idx].to_vec();
    out.extend(
        ["-o", "StrictHostKeyChecking=accept-new", "-o", "ConnectTimeout=10"]
            .iter()
            .map(|s| s.to_string()),
    );
    out.extend(tokens[ssh_idx + 1..].iter().cloned());
    out
}

/// Result of one `ssh <conn> <remote_cmd>` run.
#[derive(Clone, Debug)]
pub struct SshRunResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl SshRunResult {
    fn err(stderr: String) -> Self {
        Self { code: -1, stdout: String::new(), stderr }
    }
}

/// Run `remote_cmd` on the connection's host via its ssh command.
/// The remote command is passed as a single argument (like Nexus
/// `sshRun`); stdout/stderr are drained on reader threads so a chatty
/// remote never deadlocks the pipe, and the child is killed on timeout.
pub fn ssh_run(conn: &RemoteConnection, remote_cmd: &str, timeout_ms: u64, stdin_data: Option<&str>) -> SshRunResult {
    let tokens = tokenize_cmd(&conn.ssh_cmd);
    if tokens.is_empty() {
        return SshRunResult::err("Empty ssh command".into());
    }
    let mut cmd = Command::new(&tokens[0]);
    // v1.0.9 + round 5 (R1): connect options on every ssh spawn (see
    // ssh_connect_opts) — a first connect must never hang on the
    // interactive host-key prompt.
    let opts = ssh_connect_opts(&tokens);
    cmd.args(&opts[1..]);
    cmd.arg(remote_cmd);
    cmd.stdin(if stdin_data.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Never flash a console window on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return SshRunResult::err(e.to_string()),
    };
    if let Some(data) = stdin_data {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(data.as_bytes());
            let _ = si.flush();
        }
    }
    let out_handle = child.stdout.take();
    let err_handle = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut h) = out_handle {
            let _ = h.read_to_string(&mut s);
        }
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut h) = err_handle {
            let _ = h.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = t_out.join().unwrap_or_default();
                    let stderr = t_err.join().unwrap_or_default();
                    let mut res = SshRunResult::err(format!("timeout after {timeout_ms}ms"));
                    res.stdout = stdout;
                    res.stderr = if stderr.is_empty() { res.stderr } else { stderr };
                    return res;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return SshRunResult::err(e.to_string()),
        }
    };
    SshRunResult {
        code,
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    }
}

/// Test an (unsaved) connection draft: reachability + agent presence.
pub fn test_remote(conn: &RemoteConnection) -> RemoteTestResult {
    let ping = ssh_run(conn, "echo __MISTRALDECK_OK__", 12_000, None);
    if ping.code != 0 || !ping.stdout.contains("__MISTRALDECK_OK__") {
        let err = {
            let e = ping.stderr.trim();
            if !e.is_empty() {
                e
            } else {
                ping.stdout.trim()
            }
            .chars()
            .take(300)
            .collect::<String>()
        };
        return RemoteTestResult {
            reachable: false,
            agent_installed: false,
            agent_path: None,
            version: None,
            error: Some(if err.is_empty() { "SSH connection failed".into() } else { err }),
        };
    }
    // Login shell: non-interactive ssh sessions do not include ~/.local/bin
    // in PATH (uv tool installs live there).
    let which = ssh_run(conn, "bash -lc \"command -v vibe-app-server\"", 10_000, None);
    let agent_path = if which.code == 0 { which.stdout.trim().to_string() } else { String::new() };
    if agent_path.is_empty() {
        return RemoteTestResult {
            reachable: true,
            agent_installed: false,
            agent_path: None,
            version: None,
            error: None,
        };
    }
    let ver = ssh_run(conn, "bash -lc \"vibe-app-server --version\" 2>/dev/null", 10_000, None);
    RemoteTestResult {
        reachable: true,
        agent_installed: true,
        agent_path: Some(agent_path),
        version: Some(ver.stdout.trim().to_string()).filter(|v| !v.is_empty()),
        error: None,
    }
}

const DEFAULT_INSTALL_CMD: &str = "sh -c 'if command -v uv >/dev/null 2>&1; then uv tool install mistral-vibe 2>/dev/null || uv tool upgrade mistral-vibe; else curl -LsSf https://astral.sh/uv/install.sh | sh && $HOME/.local/bin/uv tool install mistral-vibe; fi'";

/// Install (or upgrade) the Mistral Vibe CLI on the remote host.
pub fn install_remote_agent(conn: &RemoteConnection) -> RemoteInstallResult {
    let cmd = conn.install_command.trim();
    let cmd = if cmd.is_empty() { DEFAULT_INSTALL_CMD } else { cmd };
    let res = ssh_run(conn, cmd, 300_000, None);
    let ok = res.code == 0;
    let log = format!("{}{}", res.stdout, res.stderr);
    RemoteInstallResult {
        ok,
        error: if ok {
            None
        } else {
            let e = res.stderr.trim();
            Some(if e.is_empty() { "install failed".into() } else { e.chars().take(400).collect() })
        },
        log: log.chars().take(2000).collect(),
    }
}

/// Quick liveness check for the sidebar dot.
pub fn check_remote_status(conn: &RemoteConnection) -> RemoteStatus {
    let t0 = Instant::now();
    let res = ssh_run(conn, "true", 8_000, None);
    RemoteStatus {
        id: conn.id.clone(),
        ok: res.code == 0,
        checked_at_ms: halny_vibe_deck_core::types::now_ms(),
        latency_ms: if res.code == 0 { Some(t0.elapsed().as_millis() as u64) } else { None },
    }
}

/// The agent argv for a remote slot: tokenized ssh command plus the
/// remote command that starts vibe-app-server in a login shell (so
/// uv-installed binaries are on PATH). The client then speaks JSON-RPC
/// over the ssh pipe exactly like a local process.
pub fn remote_argv(conn: &RemoteConnection) -> Vec<String> {
    // Round 5 (R1): connect options here too — the remote slot's agent
    // spawn goes through this argv.
    let mut argv = ssh_connect_opts(&tokenize_cmd(&conn.ssh_cmd));
    argv.push("bash -lc \"exec vibe-app-server\"".to_string());
    argv
}

/// List CLI sessions on the remote host by speaking JSON-RPC to a
/// vibe-app-server spawned over ssh (initialize -> session/list with cursor
/// pagination, port of Nexus `listRemoteSessions`).
pub fn list_remote_sessions(conn: &RemoteConnection) -> Vec<RemoteSessionInfo> {
    // Round 5 (R1): connect options on this spawn too — without them the
    // host-key prompt hung the lister until its deadline and the sidebar
    // showed no SSH sessions at all.
    let tokens = ssh_connect_opts(&tokenize_cmd(&conn.ssh_cmd));
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut cmd = Command::new(&tokens[0]);
    cmd.args(&tokens[1..]).arg("bash -lc \"exec vibe-app-server\"");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // stdout reader forwards JSON lines over a channel; the main loop
    // drives the handshake with a hard deadline.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    let out_handle = child.stdout.take();
    std::thread::spawn(move || {
        if let Some(mut h) = out_handle {
            let mut buf = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                match h.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if byte[0] == b'\n' {
                            let line = String::from_utf8_lossy(&buf).trim().to_string();
                            buf.clear();
                            if !line.is_empty() && line_tx.send(line).is_err() {
                                break;
                            }
                        } else {
                            buf.push(byte[0]);
                            if buf.len() > 1_000_000 {
                                buf.clear(); // runaway line: drop it
                            }
                        }
                    }
                }
            }
        }
    });
    let mut stdin = child.stdin.take();

    fn write_line(line: &str, stdin: &mut Option<std::process::ChildStdin>) -> bool {
        if let Some(si) = stdin.as_mut() {
            si.write_all(line.as_bytes()).and_then(|_| si.write_all(b"\n")).is_ok()
        } else {
            false
        }
    }

    fn request_list(stdin: &mut Option<std::process::ChildStdin>, pages: u32, cursor: &str) -> String {
        let list_id = format!("list-{pages}");
        let mut params = serde_json::Map::new();
        if !cursor.is_empty() {
            params.insert("cursor".into(), serde_json::json!(cursor));
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": list_id,
            "method": "session/list",
            "params": params,
        });
        let _ = write_line(&req.to_string(), stdin);
        list_id
    }

    let deadline = Instant::now() + Duration::from_millis(25_000);
    let mut sessions: Vec<RemoteSessionInfo> = Vec::new();
    let mut list_id = String::new();
    let mut cursor = String::new();
    let mut pages = 0u32;

    // Kick off the handshake.
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "init",
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": { "name": "Halny Vibe Deck", "version": "0.8.3" }
        },
    });
    if !write_line(&init_req.to_string(), &mut stdin) {
        let _ = child.kill();
        let _ = child.wait();
        return sessions;
    }

    loop {
        if Instant::now() >= deadline {
            break;
        }
        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                let result = msg.get("result").cloned().unwrap_or(serde_json::Value::Null);
                if id == "init" {
                    list_id = request_list(&mut stdin, pages, &cursor);
                } else if !list_id.is_empty() && id == list_id {
                    let raw = result
                        .get("sessions")
                        .or_else(|| result.get("items"))
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for s in raw {
                        let session_id = s
                            .get("sessionId")
                            .or_else(|| s.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if session_id.is_empty() {
                            continue;
                        }
                        // Round 4 (B3): untitled sessions fall back to the
                        // first user prompt (`preview`) so the list never
                        // shows a nameless row.
                        let mut title = s
                            .get("title")
                            .or_else(|| s.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if title.trim().is_empty() {
                            title = s
                                .get("preview")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                        }
                        if title.len() > 80 {
                            title = format!("{}…", &title[..80]);
                        }
                        sessions.push(RemoteSessionInfo {
                            session_id,
                            title,
                            cwd: s
                                .get("cwd")
                                .or_else(|| s.get("workingDirectory"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            updated_at: s
                                .get("updatedAt")
                                .or_else(|| s.get("updated_at"))
                                .or_else(|| s.get("startTime"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                    let next_cursor = result
                        .get("next_cursor")
                        .or_else(|| result.get("nextCursor"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !next_cursor.is_empty() && pages < 20 {
                        pages += 1;
                        cursor = next_cursor.to_string();
                        list_id = request_list(&mut stdin, pages, &cursor);
                    } else {
                        break;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // If the handshake stalled before session/list was sent
                // (e.g. initialize answered slowly), nudge it once.
                if list_id.is_empty() {
                    list_id = request_list(&mut stdin, pages, &cursor);
                }
                if child.try_wait().map(|s| s.is_some()).unwrap_or(true) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    sessions
}

// ---------------------------------------------------------------------------
// v0.9.0 lazy remote session preview (Nexus `loadRemoteSessionPreview`)
// ---------------------------------------------------------------------------

/// Cap on chunk files fetched in phase 2 (Nexus: 48).
const REMOTE_PREVIEW_MAX_CHUNKS: usize = 48;

/// Parse the phase-2 chunk dump: `@@@CHUNK:<sha>\n<json array>` blocks -
/// every chunk file holds an array of projection entries.
fn parse_remote_chunk_blocks(out: &str) -> Vec<serde_json::Value> {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for block in out.split("@@@CHUNK:") {
        let Some(nl) = block.find('\n') else {
            continue;
        };
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(block[nl + 1..].trim()) {
            entries.extend(list);
        }
    }
    entries
}

/// Lazy remote session preview (Nexus `loadRemoteSessionPreview`): read the
/// unified session store on the remote host over SSH WITHOUT starting
/// the CLI there (lazy resume). Phase 1 locates the session dir (direct id
/// first, then a CURRENT+meta.json scan), cats the latest generation
/// manifest and the session index; phase 2 cats the last 48 chunk files.
/// `None` when the remote store has nothing usable. Blocking - call from
/// `tokio::task::spawn_blocking` in server contexts.
pub fn load_remote_session_preview(
    conn: &RemoteConnection,
    session_id: &str,
) -> Option<halny_vibe_deck_vibe::stats::SessionPreviewData> {
    use halny_vibe_deck_vibe::stats::{build_session_preview, find_index_entry};
    let sid = shq(session_id);
    let phase1 = [
        format!("D=\"$HOME\"/.vibe/logs/session/unified/{sid}"),
        "if [ ! -f \"$D/CURRENT\" ]; then".to_string(),
        "  for d in \"$HOME\"/.vibe/logs/session/unified/*/; do".to_string(),
        format!(
            "    if [ -f \"$d/CURRENT\" ] && grep -q {sid} \"$d/meta.json\" 2>/dev/null; then D=\"${{d%/}}\"; break; fi"
        ),
        "  done".to_string(),
        "fi".to_string(),
        "if [ ! -f \"$D/CURRENT\" ]; then echo \"@@@NONE\"; exit 0; fi".to_string(),
        "echo \"@@@DIR:$D\"".to_string(),
        "GEN=$(ls -1 \"$D/generations\" 2>/dev/null | sort -n | tail -1)".to_string(),
        "if [ -z \"$GEN\" ] || [ ! -f \"$D/generations/$GEN/manifest.json\" ]; then echo \"@@@NONE\"; exit 0; fi"
            .to_string(),
        "echo \"@@@MANIFEST\"".to_string(),
        "cat \"$D/generations/$GEN/manifest.json\"".to_string(),
        "echo".to_string(),
        "echo \"@@@INDEX\"".to_string(),
        "cat \"$HOME\"/.vibe/logs/session/unified/.session-index.json 2>/dev/null".to_string(),
    ]
    .join("\n");
    let out = {
        let r = ssh_run(conn, &phase1, 25000, None);
        if r.code != 0 || r.stdout.trim().is_empty() {
            return None;
        }
        r.stdout
    };
    if out.contains("@@@NONE") {
        return None;
    }
    let dir_pos = out.find("@@@DIR:")?;
    let manifest_pos = out.find("@@@MANIFEST")?;
    let index_pos = out.find("@@@INDEX")?;
    let dir = out
        .get(dir_pos + "@@@DIR:".len()..)
        .and_then(|rest| rest.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();
    let manifest: serde_json::Value = out
        .get(manifest_pos + "@@@MANIFEST".len() + 1..index_pos)
        .and_then(|s| serde_json::from_str(s.trim()).ok())?;
    let shas_all: Vec<String> = manifest
        .pointer("/projection_state/chunks")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let start = shas_all.len().saturating_sub(REMOTE_PREVIEW_MAX_CHUNKS);
    let shas: Vec<String> = shas_all[start..]
        .iter()
        .filter(|s| {
            !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
        .cloned()
        .collect();
    if shas.is_empty() || dir.is_empty() {
        return None;
    }
    let cmds: Vec<String> = shas
        .iter()
        .map(|sha| {
            let chunk = format!("{dir}/chunks/{sha}.json");
            format!("echo \"@@@CHUNK:{sha}\"; cat {}; echo", shq(&chunk))
        })
        .collect();
    let out2 = {
        let r = ssh_run(conn, &cmds.join("\n"), 60000, None);
        if r.code != 0 {
            return None;
        }
        r.stdout
    };
    let entries = parse_remote_chunk_blocks(&out2);
    let index_raw = out.get(index_pos + "@@@INDEX".len() + 1..).unwrap_or("").trim();
    let index_entry = if index_raw.is_empty() {
        None
    } else {
        serde_json::from_str::<serde_json::Value>(index_raw)
            .ok()
            .and_then(|idx| find_index_entry(&idx, session_id))
    };
    if entries.is_empty() && index_entry.is_none() {
        return None;
    }
    Some(build_session_preview(&entries, index_entry.as_ref()))
}

// ---------------------------------------------------------------------------
// Remote CLI update check (Nexus updates.ts: checkRemoteCliUpdate /
// upgradeRemoteCli / pypiLatest / extractVersion / isNewerVersion)
// ---------------------------------------------------------------------------

/// PyPI package the CLI is published under (Nexus PYPI_PACKAGE).
pub(crate) const PYPI_PACKAGE: &str = "mistral-vibe";

/// HTTP GET returning the response body via `curl` (the engine has no HTTP
/// client; curl ships on all supported targets).
pub(crate) fn http_get_json(url: &str, timeout_s: u64) -> Result<String, String> {
    let mut c = std::process::Command::new("curl");
    crate::no_window(&mut c);
    let out = c
        .args(["-fsSL", "--max-time", &timeout_s.to_string(), url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// PyPI latest-version cache: (version, fetched-at) (Nexus: 5 minutes).
static PYPI_CACHE: std::sync::Mutex<Option<(String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// Latest CLI version from PyPI with a 5-minute cache (Nexus `pypiLatest`).
pub(crate) fn pypi_latest() -> Result<String, String> {
    if let Ok(guard) = PYPI_CACHE.lock() {
        if let Some((v, at)) = guard.as_ref() {
            if at.elapsed().as_secs() < 300 {
                return Ok(v.clone());
            }
        }
    }
    let url = format!("https://pypi.org/pypi/{PYPI_PACKAGE}/json");
    let body = http_get_json(&url, 15)?;
    let v = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| e.to_string())?
        .pointer("/info/version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing info.version".to_string())?
        .to_string();
    if let Ok(mut guard) = PYPI_CACHE.lock() {
        *guard = Some((v.clone(), std::time::Instant::now()));
    }
    Ok(v)
}

/// First numeric run containing a dot (Nexus `extractVersion` regex
/// `\d+\.\d+(\.\d+)?`); trailing dots trimmed. `port 8080 open` -> None.
pub fn extract_version(s: &str) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            cur.push(ch);
        } else if !cur.is_empty() {
            candidates.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        candidates.push(cur);
    }
    candidates
        .into_iter()
        .map(|c| c.trim_matches('.').to_string())
        .find(|c| c.contains('.') && c.chars().any(|x| x.is_ascii_digit()))
}

/// Numeric segment comparison (Nexus `isNewerVersion`): true when `latest`
/// is strictly newer than `current` (3.1.0 > 3.0.2, 4.0 > 3.9.9).
pub fn is_newer_version(latest: &str, current: &str) -> bool {
    // JS parseInt semantics: per dot-segment take the leading digits, so
    // release tags like "0.8.3-dioxus.7" compare as [0, 8, 3, 7].
    let parse = |v: &str| -> Vec<i64> {
        v.split('.')
            .map(|p| {
                let digits: String =
                    p.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i64>().unwrap_or(0)
            })
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    let n = l.len().max(c.len());
    for i in 0..n {
        let a = l.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// CLI update check for one remote connection (Nexus `checkRemoteCliUpdate`):
/// `vibe --version` over SSH (login shell) + PyPI latest with cache.
pub fn check_remote_cli_update(conn: &RemoteConnection) -> CliUpdateInfo {
    let ver = ssh_run(conn, "bash -lc \"vibe --version\"", 15_000, None);
    if ver.code != 0 {
        let e = ver.stderr.trim();
        let e = if e.is_empty() { "command failed".to_string() } else { e.chars().take(300).collect() };
        return CliUpdateInfo {
            current_version: None,
            latest_version: None,
            update_available: false,
            error: Some(format!("vibe not found on {}: {e}", conn.name)),
        };
    }
    let current = extract_version(&ver.stdout);
    let latest = match pypi_latest() {
        Ok(v) => Some(v),
        Err(e) => {
            return CliUpdateInfo {
                current_version: current,
                latest_version: None,
                update_available: false,
                error: Some(format!("PyPI: {}", e.chars().take(300).collect::<String>())),
            };
        }
    };
    let update_available = match (&current, &latest) {
        (Some(c), Some(l)) => is_newer_version(l, c),
        _ => false,
    };
    CliUpdateInfo {
        current_version: current,
        latest_version: latest,
        update_available,
        error: None,
    }
}

/// Upgrade command (Nexus `upgradeRemoteCli`): uv preferred, pip3 fallback.
const REMOTE_CLI_UPGRADE_CMD: &str = "bash -lc \"command -v uv >/dev/null 2>&1 && uv tool upgrade mistral-vibe || (command -v pip3 >/dev/null 2>&1 && pip3 install --user --upgrade mistral-vibe)\"";

/// Upgrade the CLI on the remote host (up to 5 minutes), then re-read
/// `vibe --version`. Blocking - call from `tokio::task::spawn_blocking`.
pub fn upgrade_remote_cli(conn: &RemoteConnection) -> CliUpgradeResult {
    let res = ssh_run(conn, REMOTE_CLI_UPGRADE_CMD, 300_000, None);
    if res.code != 0 {
        let e = format!("{}{}", res.stdout, res.stderr);
        return CliUpgradeResult {
            version: None,
            error: Some(if e.trim().is_empty() {
                "upgrade failed".to_string()
            } else {
                e.trim().chars().take(500).collect()
            }),
        };
    }
    let ver = ssh_run(conn, "bash -lc \"vibe --version\"", 15_000, None);
    CliUpgradeResult {
        version: extract_version(&ver.stdout),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_cmd("ssh user@host -p 2222"), vec!["ssh", "user@host", "-p", "2222"]);
    }

    #[test]
    fn tokenize_quotes() {
        assert_eq!(
            tokenize_cmd("ssh host \"echo 'a b'\" x"),
            vec!["ssh", "host", "echo 'a b'", "x"]
        );
    }

    #[test]
    fn shell_quote() {
        assert_eq!(shq("it's"), "'it'\\''s'");
    }

    #[test]
    fn remote_chunk_blocks_parse() {
        let out = "@@@CHUNK:abc\n[{\"type\":\"message\",\"role\":\"user\",\"content\":[]}]\n\
@@@CHUNK:def\n[{\"type\":\"reasoning\"},{\"type\":\"message\"}]";
        let entries = parse_remote_chunk_blocks(out);
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0].get("role").and_then(|r| r.as_str()),
            Some("user")
        );
        // A block without a newline (truncated output) is skipped.
        assert_eq!(parse_remote_chunk_blocks("@@@CHUNK:zzz").len(), 0);
    }

    #[test]
    fn extract_version_basic() {
        assert_eq!(extract_version("vibe 3.0.2"), Some("3.0.2".into()));
        assert_eq!(extract_version("no version here"), None);
        assert_eq!(extract_version("port 8080 open"), None);
    }

    #[test]
    fn newer_version_compare() {
        assert!(is_newer_version("3.1.0", "3.0.2"));
        assert!(!is_newer_version("3.0.2", "3.0.2"));
        assert!(is_newer_version("4.0", "3.9.9"));
    }

    #[test]
    fn newer_version_release_tags() {
        assert!(is_newer_version("0.8.3-dioxus.7", "0.8.3-dioxus.6"));
        assert!(!is_newer_version("0.8.3-dioxus.7", "0.8.3-dioxus.7"));
        assert!(is_newer_version("0.8.3-dioxus.10", "0.8.3-dioxus.7"));
        assert!(is_newer_version("0.8.3-dioxus.7", "0.8.3"));
    }
}
