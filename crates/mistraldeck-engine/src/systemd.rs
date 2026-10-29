//! Minimal systemd integration (stable v1.0.19/1.0.29/1.0.30/1.0.31 parity).
//!
//! The Electron edition needed a cgroup self-heal because Chromium migrates
//! the browser process into a transient `app-*.scope`, so sd_notify messages
//! were attributed to the wrong cgroup. The Rust edition is a single process
//! that never leaves the unit cgroup, so the plain `sd_notify(3)` protocol
//! (a unix datagram to `$NOTIFY_SOCKET`) is enough:
//!
//! - `READY=1` once the server is listening (`Type=notify`),
//! - `WATCHDOG=1` every `WATCHDOG_USEC / 2` (`WatchdogSec=`),
//! - `systemctl --user restart <unit>` for self-update so systemd
//!   re-executes the freshly installed binary inside the unit
//!   (v1.0.29: spawning the exe directly under systemd is a trap — the
//!   child dies with the unit cgroup and `exit(0)` reads as success).
//!
//! Everything is a no-op outside systemd (no `NOTIFY_SOCKET`).

use std::time::Duration;

/// True when the process was started by a systemd unit with `Type=notify`.
pub fn under_systemd() -> bool {
    std::env::var_os("NOTIFY_SOCKET").is_some()
}

#[cfg(unix)]
fn notify_socket() -> Option<std::os::unix::net::UnixDatagram> {
    let path = std::env::var("NOTIFY_SOCKET").ok()?;
    if path.is_empty() {
        return None;
    }
    let sock = std::os::unix::net::UnixDatagram::unbound().ok()?;
    // Abstract namespace sockets are passed with a leading '@'.
    let target = if let Some(rest) = path.strip_prefix('@') {
        format!("\0{rest}")
    } else {
        path
    };
    sock.connect(&target).ok()?;
    Some(sock)
}

/// `sd_notify(3)`: send a raw state line; silently ignored outside systemd.
pub fn notify_state(state: &str) {
    #[cfg(unix)]
    if let Some(sock) = notify_socket() {
        let _ = sock.send(state.as_bytes());
    }
    #[cfg(not(unix))]
    let _ = state;
}

/// Ping `WATCHDOG=1` in the background when `WatchdogSec=` is configured.
/// Returns immediately; the thread exits with the process.
pub fn start_watchdog() {
    let Some(usec) = std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    else {
        return;
    };
    if usec == 0 || !under_systemd() {
        return;
    }
    let interval = Duration::from_micros((usec / 2).max(500_000));
    std::thread::spawn(move || loop {
        std::thread::sleep(interval);
        notify_state("WATCHDOG=1");
    });
}

/// The active Halny Vibe Deck user unit, if any (`HALNY_VIBE_DECK_UNIT`
/// overrides the built-in candidate list).
pub fn active_unit() -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(u) = std::env::var("HALNY_VIBE_DECK_UNIT") {
        if !u.trim().is_empty() {
            candidates.push(u.trim().to_string());
        }
    }
    for name in ["halny-vibe-deck", "halny-vibe-deck-appserver"] {
        candidates.push(name.to_string());
    }
    candidates.into_iter().find(|unit| {
        let mut c = std::process::Command::new("systemctl");
        crate::no_window(&mut c);
        c.args(["--user", "is-active", "--quiet", unit])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Ask systemd to restart the unit so the freshly installed binary runs
/// inside the unit. Detached: the caller exits right after.
pub fn restart_unit(unit: &str) -> bool {
    let mut c = std::process::Command::new("systemctl");
    crate::no_window(&mut c);
    c.args(["--user", "restart", unit])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| true)
        .unwrap_or(false)
}
