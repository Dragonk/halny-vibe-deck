//! App + local CLI update system (Nexus `src/main/updates.ts` port, minus
//! the remote-CLI parts which live in `remote.rs` since stage C3).
//!
//! HTTP goes through `curl` subprocesses (the engine has no HTTP client):
//! version checks hit GitHub releases (the app) and PyPI (the CLI).

use std::io::Read;

use mistraldeck_core::snapshot::{
    AppUpdateInfo, AppUpdateResult, CliUpdateInfo, CliUpgradeResult, UpdateProgress,
};

use crate::remote::{
    extract_version, http_get_json, is_newer_version, pypi_latest, PYPI_PACKAGE,
};

/// GitHub repo hosting Mistral Deck releases (Nexus GITHUB_REPO).
const GITHUB_REPO: &str = "Dragonk/mistraldeck";

/// Version of this build: release builds embed the release tag through
/// `MISTRALDECK_BUILD_VERSION` (see release-dioxus.yml); dev builds fall back
/// to the crate version.
pub fn app_version() -> String {
    option_env!("MISTRALDECK_BUILD_VERSION")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .trim_start_matches('v')
        .to_string()
}

/// How this app was deployed (Nexus `appFlavor`): release bundles ship the
/// binary as `mistraldeck` (Linux) / `MistralDeck.exe` (Windows) next to a
/// `public/` directory. A `cargo run` development build has the same exe
/// name on Linux, so the `public/` directory is what actually tells a
/// bundled build from a dev build; dev builds cannot self-update.
pub fn app_flavor() -> &'static str {
    let exe = std::env::current_exe().ok().and_then(|p| {
        p.file_name().map(|n| n.to_string_lossy().to_string())
    });
    match exe.as_deref() {
        Some("mistraldeck") | Some("mistraldeck.exe") | Some("MistralDeck")
        | Some("MistralDeck.exe") => {
            let has_public = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("public").is_dir()))
                .unwrap_or(false);
            if has_public { "portable" } else { "dev" }
        }
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
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "macos"
    };
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

// ---------------------------------------------------------------------------
// App self-update (Nexus `updateApp`)
// ---------------------------------------------------------------------------

/// Shared update state the UI polls: download/install progress and the
/// terminal error of a failed update.
#[derive(Clone)]
struct UpdateState {
    progress: Option<UpdateProgress>,
    error: Option<String>,
    running: bool,
}

static UPDATE_STATE: std::sync::Mutex<Option<UpdateState>> = std::sync::Mutex::new(None);

fn set_progress(p: UpdateProgress) {
    if let Ok(mut guard) = UPDATE_STATE.lock() {
        let st = guard.get_or_insert_with(|| UpdateState {
            progress: None,
            error: None,
            running: true,
        });
        st.progress = Some(p);
        st.running = true;
    }
}

fn set_error(e: String) {
    if let Ok(mut guard) = UPDATE_STATE.lock() {
        let st = guard.get_or_insert_with(|| UpdateState {
            progress: None,
            error: None,
            running: false,
        });
        st.error = Some(e);
        st.running = false;
    }
}

/// Current self-update progress + terminal error (poll from the UI).
pub fn update_progress() -> (Option<UpdateProgress>, Option<String>) {
    if let Ok(guard) = UPDATE_STATE.lock() {
        if let Some(st) = guard.as_ref() {
            return (st.progress.clone(), st.error.clone());
        }
    }
    (None, None)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// `floor(received / total * 100)` like Nexus `Math.floor` (None without a
/// usable total).
fn percent_of(received: u64, total: Option<u64>) -> Option<i64> {
    total.filter(|t| *t > 0).map(|t| {
        ((received as f64 / t as f64) * 100.0).floor() as i64
    })
}

/// Content-Length of the final redirect target (curl resolves redirects);
/// GitHub release assets redirect to signed S3 URLs.
fn content_length(url: &str) -> Option<u64> {
    let (code, out, _) = run_capture("curl", &["-fsSLI", "--max-time", "20", url], 25_000)?;
    if code != 0 {
        return None;
    }
    for line in out.lines() {
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            if let Ok(n) = rest.trim().parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

/// Download `url` to `dest` with throttled progress reports (Nexus
/// `downloadTo`: 250ms throttle, curl follows up to the default redirects).
fn download_to(url: &str, dest: &std::path::Path, total: Option<u64>) -> Result<(), String> {
    let dest_s = dest.to_string_lossy().to_string();
    let mut child = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "600", "-o", &dest_s, url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let report = |received: u64| {
        set_progress(UpdateProgress {
            phase: "download".to_string(),
            received,
            total,
            percent: percent_of(received, total),
        });
    };
    let mut last_report = std::time::Instant::now();
    report(0);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("Download failed (curl exit code {status})"));
                }
                let received = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                report(received);
                return Ok(());
            }
            Ok(None) => {
                if last_report.elapsed() >= std::time::Duration::from_millis(250) {
                    last_report = std::time::Instant::now();
                    let received =
                        std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                    report(received);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// App self-update (Nexus `updateApp`): download the platform asset and
/// install it per flavor. Runs on a detached thread so the server fn can
/// return immediately - the UI polls `update_progress`; on success the
/// spawned helper relaunches the app and this process exits.
pub fn update_app() -> AppUpdateResult {
    let busy = match UPDATE_STATE.lock() {
        Ok(mut guard) => {
            if let Some(st) = guard.as_ref() {
                if st.running {
                    true
                } else {
                    *guard = Some(UpdateState {
                        progress: None,
                        error: None,
                        running: true,
                    });
                    false
                }
            } else {
                *guard = Some(UpdateState {
                    progress: None,
                    error: None,
                    running: true,
                });
                false
            }
        }
        Err(_) => false,
    };
    if busy {
        return AppUpdateResult {
            ok: false,
            error: Some("Update already in progress".to_string()),
        };
    }
    std::thread::spawn(|| {
        if let Err(e) = update_app_inner() {
            set_error(e);
        }
    });
    AppUpdateResult { ok: true, error: None }
}

fn update_app_inner() -> Result<(), String> {
    let flavor = app_flavor();
    if flavor == "dev" {
        return Err("Self-update is only available for installed builds".to_string());
    }
    let info = check_app_update();
    // Never downgrade: the button is gated in the UI, but the handler must
    // refuse on its own (a stale client could invoke it).
    let Some(download_url) = info.download_url.clone() else {
        return Err(info.error.clone().unwrap_or_else(|| {
            if info.update_available {
                "No update asset available for this platform".to_string()
            } else {
                "Already up to date".to_string()
            }
        }));
    };
    let tmp = std::env::temp_dir().join(format!("mistraldeck-update-{}", now_ms()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .ok_or_else(|| "current_exe unavailable".to_string())?;
    #[cfg(target_os = "windows")]
    return update_app_windows(&download_url, &tmp, &install_dir);
    #[cfg(target_os = "linux")]
    return update_app_linux(&download_url, &tmp, &install_dir);
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    return Err("Unsupported platform for self-update".to_string());
}

/// Windows portable flavor (v0.13.0/v0.13.2 flow): the detached PowerShell
/// helper does ALL the heavy work (extract, robocopy, verify, rollback,
/// relaunch) after this process exits.
#[cfg(target_os = "windows")]
fn update_app_windows(
    download_url: &str,
    tmp: &std::path::Path,
    install_dir: &std::path::Path,
) -> Result<(), String> {
    let zip_path = tmp.join("mistraldeck-portable.zip");
    let total = content_length(download_url);
    download_to(download_url, &zip_path, total)?;
    let size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
    if size < 20 * 1024 * 1024 {
        return Err(format!(
            "Downloaded archive looks invalid ({} MB) - aborting update",
            size / 1024 / 1024
        ));
    }
    // Cheap sanity check before quitting: every zip starts with the PK
    // signature; a failed download (proxy error page, truncated body)
    // would otherwise only surface in the helper, after the app closed.
    let sig = {
        use std::io::Read;
        let mut f = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut b = [0u8; 2];
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        b
    };
    if sig[0] != 0x50 || sig[1] != 0x4b {
        return Err("Downloaded file is not a zip archive - aborting update".to_string());
    }
    set_progress(UpdateProgress {
        phase: "install".to_string(),
        received: size,
        total: Some(size),
        percent: Some(100),
    });

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy().to_string();
    let exe_name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let old_exe = install_dir.join(format!("mistraldeck-old-{}.exe", now_ms()));
    let old_exe_s = old_exe.to_string_lossy().to_string();
    let old_name = old_exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let extract_dir = tmp.join("extracted");
    let extract_s = extract_dir.to_string_lossy().to_string();
    let zip_s = zip_path.to_string_lossy().to_string();
    let dir_s = install_dir.to_string_lossy().to_string();
    let tmp_s = tmp.to_string_lossy().to_string();
    let log_path = std::env::temp_dir().join("mistraldeck-update.log");
    let log_s = log_path.to_string_lossy().to_string();
    let script_path = tmp.join("mistraldeck-update.ps1");
    let q = |s: &str| format!("'{s}'");
    let pid = std::process::id();
    let ps_lines: Vec<String> = vec![
        "$ErrorActionPreference = 'Continue'".to_string(),
        format!(
            "function Log($m) {{ try {{ Add-Content -Path {} -Value ((Get-Date -Format s) + ' ' + $m) }} catch {{}} }}",
            q(&log_s)
        ),
        "function Fail($m) {".to_string(),
        "  Log $m".to_string(),
        "  $relaunch = $null".to_string(),
        format!(
            "  if (Test-Path -LiteralPath {0}) {{ $relaunch = {0} }}",
            q(&exe_s)
        ),
        format!(
            "  elseif (Test-Path -LiteralPath {0}) {{ $relaunch = {0} }}",
            q(&old_exe_s)
        ),
        "  if ($relaunch) { Log ('relaunching ' + $relaunch); try { Start-Process -FilePath $relaunch -ErrorAction Stop } catch { Log ('relaunch failed: ' + $_.Exception.Message) } }".to_string(),
        "  exit 1".to_string(),
        "}".to_string(),
        format!("Log ('mistraldeck portable update start, pid {pid}')"),
        format!(
            "try {{ Wait-Process -Id {pid} -Timeout 90 -ErrorAction Stop }} catch {{ Log 'app still running after 90s, forcing stop'; Stop-Process -Id {pid} -Force -ErrorAction SilentlyContinue }}"
        ),
        "Start-Sleep -Milliseconds 1500".to_string(),
        "Log 'extracting archive'".to_string(),
        format!(
            "try {{ Expand-Archive -Path {0} -DestinationPath {1} -Force -ErrorAction Stop }} catch {{ Fail ('extract failed: ' + $_.Exception.Message) }}",
            q(&zip_s),
            q(&extract_s)
        ),
        format!("$src = {}", q(&extract_s)),
        format!(
            "if (-not (Test-Path -LiteralPath (Join-Path $src {0}))) {{",
            q(&exe_name)
        ),
        "  $sub = Get-ChildItem -LiteralPath $src -Directory | Select-Object -First 1".to_string(),
        format!(
            "  if ($sub -and (Test-Path -LiteralPath (Join-Path $sub.FullName {0}))) {{ $src = $sub.FullName; Log ('nested zip layout, using ' + $src) }}",
            q(&exe_name)
        ),
        "  else { Fail 'extracted archive does not contain the app executable' }".to_string(),
        "}".to_string(),
        "Log 'renaming old exe'".to_string(),
        format!(
            "try {{ Rename-Item -LiteralPath {0} -NewName {1} -ErrorAction Stop }} catch {{ Fail ('rename failed: ' + $_.Exception.Message) }}",
            q(&exe_s),
            q(&old_name)
        ),
        "Log 'copying new files (robocopy)'".to_string(),
        format!(
            "robocopy $src {0} /E /R:2 /W:2 /NFL /NDL /NP /NJH /NJS",
            q(&dir_s)
        ),
        "$rc = $LASTEXITCODE".to_string(),
        "Log ('robocopy exit code ' + $rc)".to_string(),
        "if ($rc -ge 8) { Fail ('robocopy failed with exit code ' + $rc) }".to_string(),
        format!("if (-not (Test-Path -LiteralPath {0})) {{", q(&exe_s)),
        "  Log 'new exe missing after copy - rolling back'".to_string(),
        format!(
            "  Rename-Item -LiteralPath {0} -NewName {1} -Force -ErrorAction SilentlyContinue",
            q(&old_exe_s),
            q(&exe_name)
        ),
        "  Fail 'rolled back to previous exe'".to_string(),
        "}".to_string(),
        "Log 'relaunching app'".to_string(),
        format!(
            "try {{ Start-Process -FilePath {0} -ErrorAction Stop }} catch {{ Fail ('relaunch failed: ' + $_.Exception.Message) }}",
            q(&exe_s)
        ),
        "Start-Sleep -Seconds 5".to_string(),
        format!(
            "try {{ Remove-Item -LiteralPath {0} -Force -ErrorAction SilentlyContinue }} catch {{}}",
            q(&old_exe_s)
        ),
        format!(
            "try {{ Remove-Item -LiteralPath {0} -Recurse -Force -ErrorAction SilentlyContinue }} catch {{}}",
            q(&tmp_s)
        ),
        "Log 'done'".to_string(),
    ];
    let ps = ps_lines.join("\n");
    // The BOM is CRITICAL (Nexus v0.13.2): Windows PowerShell 5.1 reads
    // BOM-less files as ANSI, so every non-ASCII character in the embedded
    // paths (e.g. diacritics in the user profile folder) turns into mojibake
    // and the helper fails silently.
    std::fs::write(&script_path, format!("\u{feff}{ps}")).map_err(|e| e.to_string())?;

    // v0.13.2: never depend on PATH when spawning the helper - use the
    // absolute powershell.exe location when it exists.
    let sys_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let ps_exe_abs = std::path::Path::new(&sys_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let ps_exe = if ps_exe_abs.exists() {
        ps_exe_abs.to_string_lossy().to_string()
    } else {
        "powershell.exe".to_string()
    };

    use std::os::windows::process::CommandExt;
    let spawned = std::process::Command::new(&ps_exe)
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0000_0008) // DETACHED_PROCESS
        .spawn();
    if let Err(e) = spawned {
        // Leave a trace in the log - otherwise the app quits with no
        // update and no explanation.
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_path)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{:?} powershell spawn failed: {}", std::time::SystemTime::now(), e)
            });
    }
    set_progress(UpdateProgress {
        phase: "relaunch".to_string(),
        received: 0,
        total: None,
        percent: None,
    });
    std::process::exit(0);
}

/// Linux portable flavor: the app directory is the folder holding the
/// executable (install-linux.sh layout). Replacing files of a running
/// Linux process is safe - the kernel keeps the old inode mapped.
#[cfg(target_os = "linux")]
fn update_app_linux(
    download_url: &str,
    tmp: &std::path::Path,
    install_dir: &std::path::Path,
) -> Result<(), String> {
    let archive = tmp.join("mistraldeck-linux.tar.gz");
    let total = content_length(download_url);
    download_to(download_url, &archive, total)?;
    let size = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
    if size < 20 * 1024 * 1024 {
        return Err(format!(
            "Downloaded archive looks invalid ({} MB) - aborting update",
            size / 1024 / 1024
        ));
    }
    set_progress(UpdateProgress {
        phase: "install".to_string(),
        received: size,
        total: Some(size),
        percent: Some(100),
    });
    let archive_s = archive.to_string_lossy().to_string();
    let dir_s = install_dir.to_string_lossy().to_string();
    let res = run_capture("tar", &["-xzf", &archive_s, "-C", &dir_s], 180_000);
    match res {
        Some((0, _, _)) => {}
        Some((code, out, err)) => {
            let e = format!("{out}{err}");
            return Err(format!(
                "tar failed (exit {code}): {}",
                e.trim().chars().take(500).collect::<String>()
            ));
        }
        None => return Err("tar could not run".to_string()),
    }
    set_progress(UpdateProgress {
        phase: "relaunch".to_string(),
        received: size,
        total: Some(size),
        percent: Some(100),
    });
    // Relaunch the (now replaced) executable and exit; the kernel keeps
    // the old inode mapped for the dying process.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let _ = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_pick_per_platform() {
        let assets = vec![
            ("mistraldeck-0.8.3-linux-x64.tar.gz".to_string(), "https://dl/linux".to_string()),
            ("mistraldeck-0.8.3-windows-x64.zip".to_string(), "https://dl/win".to_string()),
            ("mistraldeck-0.8.3.dmg".to_string(), "https://dl/mac".to_string()),
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
    fn percent_of_matches_nexus_floor() {
        assert_eq!(percent_of(50, Some(200)), Some(25));
        assert_eq!(percent_of(3, Some(4)), Some(75));
        assert_eq!(percent_of(10, None), None);
        assert_eq!(percent_of(10, Some(0)), None);
        assert_eq!(percent_of(300, Some(300)), Some(100));
    }

    #[test]
    fn find_on_path_missing_binary() {
        assert_eq!(find_on_path("definitely-not-a-real-bin-xqz"), None);
    }
}
