//! App + local CLI update system (Nexus `src/main/updates.ts` port, minus
//! the remote-CLI parts which live in `remote.rs` since stage C3).
//!
//! HTTP goes through `curl` subprocesses (the engine has no HTTP client):
//! version checks hit GitHub releases (the app) and PyPI (the CLI).

use std::io::Read;

use vibedeck_core::snapshot::{AppUpdateInfo, CliUpdateInfo, CliUpgradeResult};

use crate::remote::{
    extract_version, http_get_json, is_newer_version, pypi_latest, PYPI_PACKAGE,
};

/// GitHub repo hosting VibeDeck releases (Nexus GITHUB_REPO).
const GITHUB_REPO: &str = "Dragonk/vibedeck";

/// Version of this build: release builds embed the release tag through
/// `VIBEDECK_BUILD_VERSION` (see release-dioxus.yml); dev builds fall back
/// to the crate version.
pub fn app_version() -> String {
    option_env!("VIBEDECK_BUILD_VERSION")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .trim_start_matches('v')
        .to_string()
}

/// How this app was deployed (Nexus `appFlavor`): release bundles ship the
/// binary as `vibedeck`/`vibedeck.exe` next to `public/`; a `cargo run`
/// development build keeps the crate name and cannot self-update.
pub fn app_flavor() -> &'static str {
    let exe = std::env::current_exe().ok().and_then(|p| {
        p.file_name().map(|n| n.to_string_lossy().to_string())
    });
    match exe.as_deref() {
        Some("vibedeck") | Some("vibedeck.exe") => "portable",
        _ => "dev",
    }
}

/// Pick the platform asset from a release's `(name, url)` asset list.
fn pick_asset_url(assets: &[(String, String)], os: &str) -> Option<String> {
    let suffix = match os {
        "windows" => "-windows-x64.zip",
        "linux" => "-linux-x64.tar.gz",
        // Unsupported platform (Nexus: no asset match, download stays null).
        _ => return None,
    };
    assets
        .iter()
        .find(|(name, _)| name.to_lowercase().ends_with(suffix))
        .map(|(_, url)| url.clone())
}

/// Check GitHub for a newer app release (Nexus `checkAppUpdate`).
/// Blocking - call from `tokio::task::spawn_blocking`.
pub fn check_app_update() -> AppUpdateInfo {
    let current_version = app_version();
    let flavor = app_flavor().to_string();
    let os = if cfg!(target_os = "windows") { "windows" } else { "linux" };
    match check_app_update_inner(&current_version, &flavor, os) {
        Ok(info) => info,
        Err(e) => AppUpdateInfo {
            current_version,
            flavor,
            latest_version: None,
            update_available: false,
            release_url: None,
            download_url: None,
            error: Some(e.chars().take(300).collect()),
        },
    }
}

fn check_app_update_inner(
    current_version: &str,
    flavor: &str,
    os: &str,
) -> Result<AppUpdateInfo, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let body = http_get_json(&url, 15)?;
    let rel: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = rel
        .pointer("/tag_name")
        .and_then(|v| v.as_str())
        .map(|t| t.trim_start_matches('v').to_string());
    let release_url = rel
        .pointer("/html_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut assets: Vec<(String, String)> = Vec::new();
    if let Some(list) = rel.pointer("/assets").and_then(|v| v.as_array()) {
        for a in list {
            let name = a.get("name").and_then(|v| v.as_str());
            let dl = a.get("browser_download_url").and_then(|v| v.as_str());
            if let (Some(name), Some(dl)) = (name, dl) {
                assets.push((name.to_string(), dl.to_string()));
            }
        }
    }
    let download_url = pick_asset_url(&assets, os);
    let update_available = match &tag {
        Some(latest) => is_newer_version(latest, current_version),
        None => false,
    };
    Ok(AppUpdateInfo {
        current_version: current_version.to_string(),
        flavor: flavor.to_string(),
        latest_version: tag,
        update_available,
        release_url,
        download_url,
        error: None,
    })
}

/// Run a command capturing output with a hard timeout (Nexus `runAsync`):
/// returns None only when the process could not be spawned.
fn run_capture(cmd: &str, args: &[&str], timeout_ms: u64) -> Option<(i32, String, String)> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let mut out = String::new();
                    let mut err = String::new();
                    if let Some(mut o) = child.stdout.take() {
                        let _ = o.read_to_string(&mut out);
                    }
                    if let Some(mut e) = child.stderr.take() {
                        let _ = e.read_to_string(&mut err);
                    }
                    err.push_str(" (timed out)");
                    return Some((-1, out, err));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
    Some((code, out, err))
}

/// PATH lookup for an executable (Nexus `findBinary`): login-shell
/// `command -v` on Unix (GUI launches often lack the user PATH), `where`
/// on Windows.
fn find_on_path(bin: &str) -> Option<String> {
    let out = if cfg!(target_os = "windows") {
        run_capture("cmd", &["/C", &format!("where {bin}")], 10_000)
    } else {
        run_capture("sh", &["-lc", &format!("command -v {bin}")], 10_000)
    }?;
    let (code, stdout, _) = out;
    if code != 0 {
        return None;
    }
    let first = stdout.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        None
    } else {
        Some(first)
    }
}

/// Local mistral-vibe CLI version (Nexus `localCliVersion`): `vibe` from
/// PATH first, then the sibling of the configured vibe-acp (the GUI launch
/// often lacks the shell PATH; both binaries live in the same uv tool bin).
pub fn local_cli_version(acp_path: &str) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = find_on_path("vibe") {
        candidates.push(p);
    }
    if !acp_path.is_empty() {
        if let Some(dir) = std::path::Path::new(acp_path).parent() {
            let exe = if cfg!(target_os = "windows") { "vibe.exe" } else { "vibe" };
            candidates.push(dir.join(exe).to_string_lossy().to_string());
        }
    }
    for c in candidates {
        if let Some((code, out, _)) = run_capture(&c, &["--version"], 15_000) {
            if code == 0 {
                if let Some(v) = extract_version(&out) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Local CLI update check (Nexus `checkCliUpdate`): PyPI latest with cache.
/// Blocking - call from `tokio::task::spawn_blocking`.
pub fn check_local_cli_update(acp_path: &str) -> CliUpdateInfo {
    let current = local_cli_version(acp_path);
    match pypi_latest() {
        Ok(latest) => {
            let update_available = current
                .as_ref()
                .map(|c| is_newer_version(&latest, c))
                .unwrap_or(false);
            CliUpdateInfo {
                error: if current.is_some() {
                    None
                } else {
                    Some("vibe CLI not found on this machine".to_string())
                },
                current_version: current,
                latest_version: Some(latest),
                update_available,
            }
        }
        Err(e) => CliUpdateInfo {
            current_version: current,
            latest_version: None,
            update_available: false,
            error: Some(format!("PyPI: {}", e.chars().take(300).collect::<String>())),
        },
    }
}

/// Upgrade the local CLI with uv (Nexus `upgradeLocalCli`, up to 5 minutes).
/// Blocking - call from `tokio::task::spawn_blocking`.
pub fn upgrade_local_cli(acp_path: &str) -> CliUpgradeResult {
    let Some(uv) = find_on_path("uv") else {
        return CliUpgradeResult {
            version: None,
            error: Some("uv not found - install it first (https://docs.astral.sh/uv/)".to_string()),
        };
    };
    let res = run_capture(&uv, &["tool", "upgrade", PYPI_PACKAGE], 300_000);
    let Some((code, out, err)) = res else {
        return CliUpgradeResult {
            version: None,
            error: Some("uv tool upgrade could not run".to_string()),
        };
    };
    if code != 0 {
        let e = if err.trim().is_empty() { out.trim() } else { err.trim() };
        return CliUpgradeResult {
            version: None,
            error: Some(format!(
                "uv tool upgrade failed: {}",
                e.chars().take(500).collect::<String>()
            )),
        };
    }
    CliUpgradeResult {
        version: local_cli_version(acp_path),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_pick_per_platform() {
        let assets = vec![
            ("vibedeck-0.8.3-linux-x64.tar.gz".to_string(), "https://dl/linux".to_string()),
            ("vibedeck-0.8.3-windows-x64.zip".to_string(), "https://dl/win".to_string()),
            ("vibedeck-0.8.3.dmg".to_string(), "https://dl/mac".to_string()),
        ];
        assert_eq!(pick_asset_url(&assets, "windows").as_deref(), Some("https://dl/win"));
        assert_eq!(pick_asset_url(&assets, "linux").as_deref(), Some("https://dl/linux"));
        assert_eq!(pick_asset_url(&assets, "macos"), None);
    }

    #[test]
    fn app_version_is_plain_number() {
        let v = app_version();
        assert!(v.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false), "got {v}");
    }

    #[test]
    fn find_on_path_missing_binary() {
        assert_eq!(find_on_path("definitely-not-a-real-bin-xqz"), None);
    }
}
