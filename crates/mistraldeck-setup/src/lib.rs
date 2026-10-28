//! First-run dependency handling for Halny Vibe Deck.
//!
//! The Mistral Vibe CLI is a Python tool installed with `uv`:
//! `uv tool install mistral-vibe` provides `vibe`, `vibe-app-server` and
//! the legacy `vibe-acp` entry points. This crate locates existing
//! installs (PATH + the default uv tool locations on each platform) and
//! can install/upgrade everything. The app talks to the agent through
//! `vibe-app-server` only (see `detect_appserver`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Result};

/// What the app needs to talk to the agent.
#[derive(Clone, Debug, Default)]
pub struct SetupStatus {
    pub uv_path: Option<PathBuf>,
    /// The `vibe-app-server` entry point (the only agent transport).
    pub appserver_path: Option<PathBuf>,
    pub vibe_cli_version: Option<String>,
}

/// Progress message shown in the setup view.
#[derive(Clone, Debug)]
pub enum SetupProgress {
    Info(String),
    Success(String),
    Error(String),
    Done(SetupStatus),
}

/// Candidate filesystem locations for a binary, per platform.
fn candidate_paths(name: &str) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        let app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
        vec![
            // Primary location after `uv tool install mistral-vibe`
            app_data
                .join("uv")
                .join("tools")
                .join("mistral-vibe")
                .join("Scripts")
                .join(format!("{name}.exe")),
            app_data.join("uv").join("bin").join(format!("{name}.exe")),
            home.join(".local").join("bin").join(format!("{name}.exe")),
        ]
    } else {
        vec![
            home.join(".local")
                .join("share")
                .join("uv")
                .join("tools")
                .join("mistral-vibe")
                .join("bin")
                .join(name),
            home.join(".local").join("bin").join(name),
            PathBuf::from("/usr/local/bin").join(name),
            PathBuf::from("/opt/homebrew/bin").join(name),
        ]
    }
}

/// Search PATH (plus the standard uv locations) for an executable.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = if cfg!(windows) {
                dir.join(format!("{name}.exe"))
            } else {
                dir.join(name)
            };
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    candidate_paths(name).into_iter().find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Detect everything the app needs.
pub fn check_setup() -> SetupStatus {
    let uv_path = find_binary("uv");
    let appserver_path = detect_appserver();
    let vibe_cli_version = find_binary("vibe").and_then(|vibe| {
        let mut cmd = Command::new(vibe);
        cmd.arg("--version").stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    SetupStatus {
        uv_path,
        appserver_path,
        vibe_cli_version,
    }
}

/// Locate the `vibe-app-server` binary (the agent transport the app
/// spawns): PATH first, then the standard uv tool install locations
/// (`uv tool dir` when `uv` is available, plus the fixed defaults).
pub fn detect_appserver() -> Option<PathBuf> {
    let mut found = find_binary("vibe-app-server");
    if found.is_none() {
        // `uv tool dir` covers non-default uv install roots.
        if let Some(uv) = find_binary("uv") {
            if let Ok(out) = Command::new(uv).args(["tool", "dir"]).output() {
                let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() && !dir.is_empty() {
                    let exe = if cfg!(windows) {
                        "vibe-app-server.exe"
                    } else {
                        "vibe-app-server"
                    };
                    let candidate = PathBuf::from(dir)
                        .join("mistral-vibe")
                        .join("bin")
                        .join(exe);
                    if is_executable(&candidate) {
                        found = Some(candidate);
                    }
                }
            }
        }
    }
    found
}

/// Human-readable install instructions (shown when detection fails).
pub fn install_instructions() -> Vec<String> {
    vec![
        "Install the Mistral Vibe CLI with uv:".into(),
        "  curl -LsSf https://astral.sh/uv/install.sh | sh".into(),
        "  uv tool install mistral-vibe".into(),
        "or set agent_path in the config file to an existing vibe-app-server binary.".into(),
    ]
}

fn run(cmd: &str, args: &[&str], on_line: &mut impl FnMut(SetupProgress)) -> Result<()> {
    use std::io::BufRead;
    let mut proc = Command::new(cmd);
    proc.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Never flash a console window for the child process on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        proc.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = proc
        .spawn()
        .map_err(|e| anyhow!("failed to run `{cmd} {args:?}`: {e}"))?;

    // Drain stderr in a thread (uv writes progress there); stream stdout
    // line by line so the UI sees progress immediately.
    let stderr = child.stderr.take();
    let stderr_handle = stderr.map(|stderr| {
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    println!("[setup] {trimmed}");
                }
            }
        })
    });

    if let Some(stdout) = child.stdout.take() {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                on_line(SetupProgress::Info(trimmed.to_string()));
            }
        }
    }

    let status = child.wait()?;
    if let Some(handle) = stderr_handle {
        let _ = handle.join();
    }
    if !status.success() {
        bail!("`{cmd}` exited with status {status}");
    }
    Ok(())
}

/// Install uv (if missing), then install/upgrade mistral-vibe.
/// `on_progress` receives human-readable progress updates.
pub fn install_dependencies(mut on_progress: impl FnMut(SetupProgress)) -> Result<SetupStatus> {
    let status = check_setup();

    // Step 1: uv
    let uv_path = match status.uv_path.clone() {
        Some(p) => {
            on_progress(SetupProgress::Success(format!("uv found: {}", p.display())));
            p
        }
        None => install_uv(&mut on_progress)?,
    };

    // Step 2: mistral-vibe (fresh install, or upgrade when already present).
    if find_binary("vibe").is_some() || find_binary("vibe-app-server").is_some() {
        on_progress(SetupProgress::Info("Upgrading mistral-vibe…".into()));
        run(
            uv_path.to_string_lossy().as_ref(),
            &["tool", "upgrade", "mistral-vibe"],
            &mut on_progress,
        )?;
        on_progress(SetupProgress::Success("mistral-vibe upgraded.".into()));
    } else {
        on_progress(SetupProgress::Info("Installing mistral-vibe…".into()));
        run(
            uv_path.to_string_lossy().as_ref(),
            &["tool", "install", "mistral-vibe"],
            &mut on_progress,
        )?;
        on_progress(SetupProgress::Success("mistral-vibe installed.".into()));
    }

    // Step 3: locate vibe-app-server
    let final_status = check_setup();
    match final_status.appserver_path.clone() {
        Some(p) => {
            on_progress(SetupProgress::Success(format!(
                "vibe-app-server ready: {}",
                p.display()
            )));
        }
        None => {
            on_progress(SetupProgress::Error(
                "vibe-app-server not found after installation. Restart the app or install it manually."
                    .into(),
            ));
        }
    }
    on_progress(SetupProgress::Done(final_status.clone()));
    Ok(final_status)
}

fn install_uv(on_progress: &mut impl FnMut(SetupProgress)) -> Result<PathBuf> {
    on_progress(SetupProgress::Info("Installing uv package manager…".into()));
    if cfg!(windows) {
        run(
            "powershell",
            &[
                "-ExecutionPolicy",
                "ByPass",
                "-NonInteractive",
                "-Command",
                "irm https://astral.sh/uv/install.ps1 | iex",
            ],
            on_progress,
        )?;
    } else {
        run(
            "sh",
            &["-c", "curl -LsSf https://astral.sh/uv/install.sh | sh"],
            on_progress,
        )?;
    }
    let uv = find_binary("uv").ok_or_else(|| {
        anyhow!("uv not found after installation. Restart the app or install uv manually.")
    })?;
    on_progress(SetupProgress::Success(format!(
        "uv installed: {}",
        uv.display()
    )));
    Ok(uv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_paths_are_absolute() {
        for p in candidate_paths("vibe-app-server") {
            assert!(
                p.is_absolute(),
                "candidate path not absolute: {}",
                p.display()
            );
        }
    }

    #[test]
    fn install_instructions_are_non_empty() {
        assert!(install_instructions().iter().all(|l| !l.trim().is_empty()));
    }

    #[test]
    fn finds_a_real_binary_on_path() {
        // `sh` exists on every Unix; on Windows CI this test is skipped.
        if !cfg!(windows) {
            assert!(find_binary("sh").is_some());
        }
    }
}
