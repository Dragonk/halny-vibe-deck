import { execFile, spawn } from "child_process"
import https from "https"
import os from "os"
import path from "path"
import fs from "fs"
import { app } from "electron"
import { findBinary, tryExec } from "./setup"
import { sshRun } from "./remote"
import { loadConfig } from "./config"
import type { RemoteConnection } from "./config"

// v0.9.0: update system for the app itself (GitHub releases) and the
// mistral-vibe CLI (PyPI). The renderer orchestrates session restore after a
// CLI upgrade; this module only reports versions and performs the upgrades.

const GITHUB_REPO = "Dragonk/mistral-vibe-nexus"
const PYPI_PACKAGE = "mistral-vibe"

export type AppFlavor = "dev" | "installed" | "portable" | "appimage"

export interface UpdateProgress {
  phase: "download" | "install" | "relaunch"
  received: number
  total: number | null
  percent: number | null
}

export interface AppUpdateInfo {
  currentVersion: string
  flavor: AppFlavor
  latestVersion: string | null
  updateAvailable: boolean
  releaseUrl: string | null
  downloadUrl: string | null
  error: string | null
}

export interface CliUpdateInfo {
  currentVersion: string | null
  latestVersion: string | null
  updateAvailable: boolean
  error: string | null
}

function fetchJson(url: string, timeoutMs = 15000): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      { headers: { "User-Agent": "mistral-vibe-nexus-updater", Accept: "application/json" }, timeout: timeoutMs },
      (res) => {
        if (res.statusCode && (res.statusCode < 200 || res.statusCode >= 300)) {
          res.resume()
          reject(new Error("HTTP " + res.statusCode + " for " + url))
          return
        }
        let data = ""
        res.setEncoding("utf8")
        res.on("data", (c: string) => {
          data += c
          if (data.length > 8 * 1024 * 1024) req.destroy(new Error("Response too large"))
        })
        res.on("end", () => {
          try {
            resolve(JSON.parse(data))
          } catch (err) {
            reject(err as Error)
          }
        })
      }
    )
    req.on("timeout", () => {
      req.destroy(new Error("Request timed out: " + url))
    })
    req.on("error", (err: Error) => reject(err))
  })
}

function parseVersion(v: string): number[] {
  return v.replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0)
}

export function isNewerVersion(latest: string, current: string): boolean {
  const a = parseVersion(latest)
  const b = parseVersion(current)
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const da = a[i] ?? 0
    const db = b[i] ?? 0
    if (da > db) return true
    if (da < db) return false
  }
  return false
}

/** v0.11.0: how this app was deployed - decides the self-update strategy. */
export function appFlavor(): AppFlavor {
  if (!app.isPackaged) return "dev"
  if (process.platform === "linux" && process.env.APPIMAGE) return "appimage"
  if (process.platform === "win32") {
    try {
      // NSIS-installed builds ship an uninstaller next to the exe; the
      // portable zip (win-unpacked) does not.
      const dir = path.dirname(process.execPath)
      const entries = fs.readdirSync(dir)
      if (entries.some((f) => /^uninstall.*\.exe$/i.test(f))) return "installed"
    } catch {
      /* unreadable dir - treat as portable */
    }
    return "portable"
  }
  // Linux dir layout (vibe-nexus-linux-x64.zip via install-linux.sh).
  return "portable"
}

export async function checkAppUpdate(): Promise<AppUpdateInfo> {
  const currentVersion = app.getVersion()
  const flavor = appFlavor()
  try {
    const rel = (await fetchJson("https://api.github.com/repos/" + GITHUB_REPO + "/releases/latest")) as {
      tag_name?: string
      html_url?: string
      assets?: { name: string; browser_download_url: string }[]
    }
    const latestVersion = rel.tag_name ? rel.tag_name.replace(/^v/, "") : null
    const assets = rel.assets ?? []
    let downloadUrl: string | null = null
    if (flavor === "appimage") {
      const asset = assets.find((a) => a.name.toLowerCase().endsWith(".appimage"))
      downloadUrl = asset?.browser_download_url ?? null
    } else if (process.platform === "win32") {
      if (flavor === "installed") {
        const asset = assets.find((a) => a.name.toLowerCase().endsWith(".exe") && a.name.toLowerCase().includes("setup"))
        downloadUrl = asset?.browser_download_url ?? null
      } else {
        // Portable flavor: update in place with the portable zip - running
        // the NSIS installer would install somewhere else entirely.
        const asset = assets.find((a) => a.name.toLowerCase().endsWith("portable.zip"))
        downloadUrl = asset?.browser_download_url ?? null
      }
    } else if (process.platform === "linux") {
      const asset = assets.find((a) => a.name.toLowerCase().endsWith(".zip") && a.name.toLowerCase().includes("linux"))
      downloadUrl = asset?.browser_download_url ?? null
    }
    return {
      currentVersion,
      flavor,
      latestVersion,
      updateAvailable: latestVersion ? isNewerVersion(latestVersion, currentVersion) : false,
      releaseUrl: rel.html_url ?? null,
      downloadUrl,
      error: null,
    }
  } catch (err) {
    return {
      currentVersion,
      flavor,
      latestVersion: null,
      updateAvailable: false,
      releaseUrl: null,
      downloadUrl: null,
      error: (err as Error).message,
    }
  }
}

let pypiLatestCache: { version: string; at: number } | null = null

async function pypiLatest(): Promise<string> {
  if (pypiLatestCache && Date.now() - pypiLatestCache.at < 5 * 60000) return pypiLatestCache.version
  const data = (await fetchJson("https://pypi.org/pypi/" + PYPI_PACKAGE + "/json")) as { info?: { version?: string } }
  const version = data.info?.version
  if (!version) throw new Error("PyPI response missing version")
  pypiLatestCache = { version, at: Date.now() }
  return version
}

/** Pull the semver token out of CLI output like "vibe 3.0.2". */
function extractVersion(out: string): string | null {
  const m = out.match(/\d+\.\d+(?:\.\d+)?/)
  return m ? m[0] : null
}

function localCliVersion(): string | null {
  const candidates: string[] = []
  const fromPath = findBinary("vibe")
  if (fromPath) candidates.push(fromPath)
  // v0.11.0: GUI launches often lack the user's shell PATH - the configured
  // vibe-acp sits next to the `vibe` binary in the same uv tool bin dir.
  try {
    const acpPath = loadConfig().acpPath
    if (acpPath) {
      candidates.push(path.join(path.dirname(acpPath), process.platform === "win32" ? "vibe.exe" : "vibe"))
    }
  } catch {
    /* config unavailable */
  }
  for (const candidate of candidates) {
    const out = tryExec('"' + candidate + '" --version')
    if (out) {
      const v = extractVersion(out)
      if (v) return v
    }
  }
  return null
}

export async function checkCliUpdate(): Promise<CliUpdateInfo> {
  let current: string | null = null
  try {
    current = localCliVersion()
  } catch {
    current = null
  }
  try {
    const latest = await pypiLatest()
    return {
      currentVersion: current,
      latestVersion: latest,
      updateAvailable: current ? isNewerVersion(latest, current) : false,
      error: current ? null : "vibe CLI not found on this machine",
    }
  } catch (err) {
    return { currentVersion: current, latestVersion: null, updateAvailable: false, error: (err as Error).message }
  }
}

export async function checkRemoteCliUpdate(conn: RemoteConnection): Promise<CliUpdateInfo> {
  try {
    const r = await sshRun(conn, 'bash -lc "vibe --version"', 15000)
    if (r.code !== 0) {
      throw new Error("vibe not found on " + conn.name + (r.stderr ? ": " + r.stderr.trim() : ""))
    }
    const current = extractVersion(r.stdout.trim())
    const latest = await pypiLatest()
    return {
      currentVersion: current,
      latestVersion: latest,
      updateAvailable: current ? isNewerVersion(latest, current) : false,
      error: null,
    }
  } catch (err) {
    return { currentVersion: null, latestVersion: null, updateAvailable: false, error: (err as Error).message }
  }
}


// ---------------------------------------------------------------------------
// Upgrades
// ---------------------------------------------------------------------------

function runAsync(cmd: string, args: string[], timeoutMs: number): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    let out = ""
    let err = ""
    let done = false
    let proc
    try {
      proc = spawn(cmd, args, { windowsHide: true })
    } catch (e) {
      resolve({ code: -1, stdout: "", stderr: String(e) })
      return
    }
    const timer = setTimeout(() => {
      try {
        proc.kill()
      } catch {
        /* already gone */
      }
      if (!done) {
        done = true
        resolve({ code: -1, stdout: out, stderr: err + " (timed out)" })
      }
    }, timeoutMs)
    proc.stdout?.on("data", (d: Buffer) => {
      out += d.toString()
    })
    proc.stderr?.on("data", (d: Buffer) => {
      err += d.toString()
    })
    proc.on("error", (e: Error) => {
      if (!done) {
        done = true
        clearTimeout(timer)
        resolve({ code: -1, stdout: out, stderr: err + String(e) })
      }
    })
    proc.on("exit", (code) => {
      if (!done) {
        done = true
        clearTimeout(timer)
        resolve({ code: code ?? -1, stdout: out, stderr: err })
      }
    })
  })
}

export interface CliUpgradeResult {
  version: string | null
  error: string | null
}

export async function upgradeLocalCli(): Promise<CliUpgradeResult> {
  const uv = findBinary("uv")
  if (!uv) return { version: null, error: "uv not found - install it first (https://docs.astral.sh/uv/)" }
  const r = await runAsync(uv, ["tool", "upgrade", PYPI_PACKAGE], 300000)
  if (r.code !== 0) {
    return { version: null, error: "uv tool upgrade failed: " + (r.stderr || r.stdout).trim().slice(0, 500) }
  }
  return { version: localCliVersion(), error: null }
}

export async function upgradeRemoteCli(conn: RemoteConnection): Promise<CliUpgradeResult> {
  const cmd =
    'bash -lc "command -v uv >/dev/null 2>&1 && uv tool upgrade ' +
    PYPI_PACKAGE +
    ' || (command -v pip3 >/dev/null 2>&1 && pip3 install --user --upgrade ' +
    PYPI_PACKAGE +
    ')"'
  const r = await sshRun(conn, cmd, 300000)
  if (r.code !== 0) {
    return { version: null, error: "remote upgrade failed: " + (r.stderr || r.stdout).trim().slice(0, 500) }
  }
  const v = await sshRun(conn, 'bash -lc "vibe --version"', 15000)
  return { version: v.code === 0 ? extractVersion(v.stdout.trim()) : null, error: null }
}

// ---------------------------------------------------------------------------
// App self-update
// ---------------------------------------------------------------------------

function downloadTo(
  url: string,
  dest: string,
  onProgress?: (received: number, total: number | null) => void,
  redirects = 0
): Promise<void> {
  return new Promise((resolve, reject) => {
    if (redirects > 5) {
      reject(new Error("Too many redirects while downloading"))
      return
    }
    const req = https.get(url, { headers: { "User-Agent": "mistral-vibe-nexus-updater" }, timeout: 120000 }, (res) => {
      if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume()
        const next = new URL(res.headers.location, url).toString()
        void downloadTo(next, dest, onProgress, redirects + 1).then(resolve, reject)
        return
      }
      if (res.statusCode !== 200) {
        res.resume()
        reject(new Error("Download failed: HTTP " + res.statusCode))
        return
      }
      const totalHeader = res.headers["content-length"]
      const total = totalHeader ? parseInt(String(totalHeader), 10) || null : null
      let received = 0
      let lastReportAt = 0
      res.on("data", (chunk: Buffer) => {
        received += chunk.length
        const now = Date.now()
        // v0.11.0: throttled progress so long downloads are visible.
        if (onProgress && now - lastReportAt > 250) {
          lastReportAt = now
          onProgress(received, total)
        }
      })
      const f = fs.createWriteStream(dest)
      res.pipe(f)
      f.on("finish", () => {
        if (onProgress) onProgress(received, total)
        f.close(() => resolve())
      })
      f.on("error", reject)
      res.on("error", reject)
    })
    req.on("timeout", () => {
      req.destroy(new Error("Download timed out"))
    })
    req.on("error", reject)
  })
}

export interface AppUpdateResult {
  ok: boolean
  error: string | null
}

export async function updateApp(onProgress?: (p: UpdateProgress) => void): Promise<AppUpdateResult> {
  if (!app.isPackaged) {
    return { ok: false, error: "Self-update is only available for installed builds" }
  }
  const flavor = appFlavor()
  const info = await checkAppUpdate()
  if (!info.updateAvailable || !info.downloadUrl) {
    // v0.11.0: never downgrade - the button is gated in the UI, but the
    // handler must refuse on its own (a stale renderer could invoke it).
    return {
      ok: false,
      error:
        info.error ??
        (info.updateAvailable ? "No update asset available for this platform" : "Already up to date"),
    }
  }
  const emit = (p: UpdateProgress): void => {
    try {
      onProgress?.(p)
    } catch {
      /* progress reporting must never break the update */
    }
  }
  const dlProgress = (received: number, total: number | null): void =>
    emit({ phase: "download", received, total, percent: total ? Math.floor((received / total) * 100) : null })
  const q = (s: string): string => "'" + s + "'"
  const tmp = path.join(os.tmpdir(), "nexus-update-" + Date.now())
  fs.mkdirSync(tmp, { recursive: true })
  try {
    if (flavor === "appimage") {
      const dest = path.join(tmp, "nexus.AppImage")
      await downloadTo(info.downloadUrl, dest, dlProgress)
      const size = fs.statSync(dest).size
      if (size < 20 * 1024 * 1024) {
        return { ok: false, error: "Downloaded AppImage looks invalid (" + Math.round(size / 1024 / 1024) + " MB) - aborting update" }
      }
      const target = process.env.APPIMAGE
      if (!target) {
        return { ok: false, error: "APPIMAGE env missing - cannot self-update" }
      }
      fs.chmodSync(dest, 0o755)
      emit({ phase: "install", received: size, total: size, percent: 100 })
      // Replacing the AppImage file of a running app is safe: the mount
      // keeps the old inode mapped, the next launch uses the new file.
      fs.renameSync(dest, target)
      emit({ phase: "relaunch", received: size, total: size, percent: 100 })
      app.relaunch()
      app.exit(0)
      return { ok: true, error: null }
    }
    if (process.platform === "win32" && flavor === "installed") {
      const installer = path.join(tmp, "nexus-setup.exe")
      await downloadTo(info.downloadUrl, installer, dlProgress)
      const size = fs.statSync(installer).size
      if (size < 20 * 1024 * 1024) {
        return {
          ok: false,
          error: "Downloaded installer looks invalid (" + Math.round(size / 1024) + " KiB) - aborting update",
        }
      }
      // The NSIS installer is an assisted one (oneClick=false): in silent mode
      // (/S) it does NOT close a running app and aborts on locked files.
      // Previous versions launched it while the app was still shutting down,
      // which silently failed the install and left the app closed. The helper
      // script now waits for THIS process to fully exit before running the
      // installer, then relaunches the same executable path. Every step is
      // appended to nexus-update.log in the temp directory for diagnostics.
      const exe = process.execPath
      const logPath = path.join(os.tmpdir(), "nexus-update.log")
      const scriptPath = path.join(tmp, "nexus-update.ps1")
      const ps = [
        "$ErrorActionPreference = 'Continue'",
        "function Log($m) { try { Add-Content -Path " + q(logPath) + " -Value ((Get-Date -Format s) + ' ' + $m) } catch {} }",
        "Log ('nexus update start, pid " + process.pid + ", installer ' + " + q(installer) + ")",
        "try { Wait-Process -Id " + process.pid + " -Timeout 90 -ErrorAction Stop } catch { Log 'app still running after 90s, forcing stop'; Stop-Process -Id " + process.pid + " -Force -ErrorAction SilentlyContinue }",
        "Start-Sleep -Milliseconds 1500",
        "Log 'launching installer'",
        "$p = Start-Process -FilePath " + q(installer) + " -ArgumentList '/S' -Wait -PassThru",
        "Log ('installer exit code ' + $p.ExitCode)",
        "Log 'relaunching app'",
        "Start-Process -FilePath " + q(exe),
        "Log 'done'",
      ].join("\n")
      fs.writeFileSync(scriptPath, ps, "utf8")
      emit({ phase: "install", received: fs.statSync(installer).size, total: fs.statSync(installer).size, percent: 100 })
      spawn(
        "powershell.exe",
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath],
        { detached: true, stdio: "ignore", windowsHide: true }
      ).unref()
      emit({ phase: "relaunch", received: 0, total: null, percent: null })
      app.quit()
      return { ok: true, error: null }
    }
    if (process.platform === "win32") {
      // v0.13.0: portable flavor - the detached helper does ALL the heavy
      // work (extract, copy, verify, relaunch) after this process exits.
      // v0.12.0 extracted in-process and copied with Copy-Item, whose
      // wildcard merge semantics are unreliable when destination
      // subdirectories (resources/, locales/) already exist; a silent
      // copy failure left the app closed with no relaunch and no updated
      // files. The helper now uses robocopy, verifies the new exe exists
      // and rolls back to the renamed previous exe on any failure.
      const zipPath = path.join(tmp, "nexus-portable.zip")
      await downloadTo(info.downloadUrl, zipPath, dlProgress)
      const size = fs.statSync(zipPath).size
      if (size < 20 * 1024 * 1024) {
        return { ok: false, error: "Downloaded archive looks invalid (" + Math.round(size / 1024 / 1024) + " MB) - aborting update" }
      }
      // Cheap sanity check before quitting: every zip starts with the PK
      // signature. A failed download (proxy error page, truncated body)
      // would otherwise only surface in the helper, after the app closed.
      const fd = fs.openSync(zipPath, "r")
      const sig = Buffer.alloc(2)
      fs.readSync(fd, sig, 0, 2, 0)
      fs.closeSync(fd)
      if (sig[0] !== 0x50 || sig[1] !== 0x4b) {
        return { ok: false, error: "Downloaded file is not a zip archive - aborting update" }
      }
      emit({ phase: "install", received: size, total: size, percent: 100 })
      const exe = process.execPath
      const exeName = path.basename(exe)
      const appDir = path.dirname(exe)
      const oldExe = path.join(appDir, "nexus-old-" + Date.now() + ".exe")
      const extractDir = path.join(tmp, "extracted")
      const logPath = path.join(os.tmpdir(), "nexus-update.log")
      const scriptPath = path.join(tmp, "nexus-update.ps1")
      const ps = [
        "$ErrorActionPreference = 'Continue'",
        "function Log($m) { try { Add-Content -Path " + q(logPath) + " -Value ((Get-Date -Format s) + ' ' + $m) } catch {} }",
        "function Fail($m) {",
        "  Log $m",
        "  $relaunch = $null",
        "  if (Test-Path -LiteralPath " + q(exe) + ") { $relaunch = " + q(exe) + " }",
        "  elseif (Test-Path -LiteralPath " + q(oldExe) + ") { $relaunch = " + q(oldExe) + " }",
        "  if ($relaunch) { Log ('relaunching ' + $relaunch); try { Start-Process -FilePath $relaunch -ErrorAction Stop } catch { Log ('relaunch failed: ' + $_.Exception.Message) } }",
        "  exit 1",
        "}",
        "Log ('nexus portable update start, pid " + process.pid + "')",
        "try { Wait-Process -Id " + process.pid + " -Timeout 90 -ErrorAction Stop } catch { Log 'app still running after 90s, forcing stop'; Stop-Process -Id " + process.pid + " -Force -ErrorAction SilentlyContinue }",
        "Start-Sleep -Milliseconds 1500",
        "Log 'extracting archive'",
        "try { Expand-Archive -Path " + q(zipPath) + " -DestinationPath " + q(extractDir) + " -Force -ErrorAction Stop } catch { Fail ('extract failed: ' + $_.Exception.Message) }",
        "$src = " + q(extractDir),
        "if (-not (Test-Path -LiteralPath (Join-Path $src " + q(exeName) + "))) {",
        "  $sub = Get-ChildItem -LiteralPath $src -Directory | Select-Object -First 1",
        "  if ($sub -and (Test-Path -LiteralPath (Join-Path $sub.FullName " + q(exeName) + "))) { $src = $sub.FullName; Log ('nested zip layout, using ' + $src) }",
        "  else { Fail 'extracted archive does not contain the app executable' }",
        "}",
        "Log 'renaming old exe'",
        "try { Rename-Item -LiteralPath " + q(exe) + " -NewName " + q(path.basename(oldExe)) + " -ErrorAction Stop } catch { Fail ('rename failed: ' + $_.Exception.Message) }",
        "Log 'copying new files (robocopy)'",
        "robocopy $src " + q(appDir) + " /E /R:2 /W:2 /NFL /NDL /NP /NJH /NJS",
        "$rc = $LASTEXITCODE",
        "Log ('robocopy exit code ' + $rc)",
        "if ($rc -ge 8) { Fail ('robocopy failed with exit code ' + $rc) }",
        "if (-not (Test-Path -LiteralPath " + q(exe) + ")) {",
        "  Log 'new exe missing after copy - rolling back'",
        "  Rename-Item -LiteralPath " + q(oldExe) + " -NewName " + q(exeName) + " -Force -ErrorAction SilentlyContinue",
        "  Fail 'rolled back to previous exe'",
        "}",
        "Log 'relaunching app'",
        "try { Start-Process -FilePath " + q(exe) + " -ErrorAction Stop } catch { Fail ('relaunch failed: ' + $_.Exception.Message) }",
        "Start-Sleep -Seconds 5",
        "try { Remove-Item -LiteralPath " + q(oldExe) + " -Force -ErrorAction SilentlyContinue } catch {}",
        "try { Remove-Item -LiteralPath " + q(tmp) + " -Recurse -Force -ErrorAction SilentlyContinue } catch {}",
        "Log 'done'",
      ].join("\n")
      fs.writeFileSync(scriptPath, ps, "utf8")
      spawn(
        "powershell.exe",
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath],
        { detached: true, stdio: "ignore", windowsHide: true }
      ).unref()
      emit({ phase: "relaunch", received: 0, total: null, percent: null })
      app.quit()
      return { ok: true, error: null }
    }
    if (process.platform === "linux") {
      const zipPath = path.join(tmp, "nexus-linux.zip")
      await downloadTo(info.downloadUrl, zipPath, dlProgress)
      const size = fs.statSync(zipPath).size
      if (size < 20 * 1024 * 1024) {
        return {
          ok: false,
          error: "Downloaded archive looks invalid (" + Math.round(size / 1024) + " KiB) - aborting update",
        }
      }
      // User-dir layout (scripts/install-linux.sh): the app directory is the
      // folder holding the executable. Replacing files of a running Linux
      // process is safe - the kernel keeps the old inode mapped.
      const installDir = path.dirname(process.execPath)
      emit({ phase: "install", received: size, total: size, percent: 100 })
      const r = await runAsync("unzip", ["-o", zipPath, "-d", installDir], 180000)
      if (r.code !== 0) {
        return { ok: false, error: "unzip failed: " + (r.stderr || r.stdout).trim().slice(0, 500) }
      }
      emit({ phase: "relaunch", received: size, total: size, percent: 100 })
      app.relaunch()
      app.exit(0)
      return { ok: true, error: null }
    }
    return { ok: false, error: "Unsupported platform for self-update: " + process.platform }
  } catch (err) {
    return { ok: false, error: (err as Error).message }
  }
}
