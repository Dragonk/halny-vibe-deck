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
import { buildInstalledUpdateCmd, extractZip, locatePayloadExe } from "./updateCmd"

// v0.9.0: update system for the app itself (GitHub releases) and the
// mistral-vibe CLI (PyPI). The renderer orchestrates session restore after a
// CLI upgrade; this module only reports versions and performs the upgrades.

const GITHUB_REPO = "Dragonk/halny-vibe-deck"
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
      { headers: { "User-Agent": "halny-vibe-deck-updater", Accept: "application/json" }, timeout: timeoutMs },
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
  // Linux dir layout (halny-vibe-deck-linux-x64.zip via install-linux.sh).
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
    const req = https.get(url, { headers: { "User-Agent": "halny-vibe-deck-updater" }, timeout: 120000 }, (res) => {
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

// v0.13.8: spawn the generated update script with cmd.exe, detached so it
// survives app.quit(). /d skips AutoRun registry hooks, /s stops cmd from
// mangling the quoted script path.
function spawnCmdHelper(scriptPath: string, logPath: string): void {
  const sysRoot = process.env.SystemRoot ?? "C:\\Windows"
  const cmdExe = path.join(sysRoot, "System32", "cmd.exe")
  try {
    spawn(cmdExe, ["/d", "/s", "/c", scriptPath], {
      detached: true,
      stdio: "ignore",
      windowsHide: true,
    })
      .on("error", (e) => {
        // If cmd.exe cannot even start, leave a trace in the log -
        // otherwise the app quits with no update and no explanation.
        try {
          fs.appendFileSync(logPath, new Date().toISOString() + " cmd spawn failed: " + String(e) + "\n")
        } catch {
          /* nothing else to do - we are quitting */
        }
      })
      .unref()
  } catch (e) {
    try {
      fs.appendFileSync(logPath, new Date().toISOString() + " cmd spawn threw: " + String(e) + "\n")
    } catch {
      /* nothing else to do - we are quitting */
    }
  }
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
      // v0.13.8: the NSIS installer must not run while the app is alive
      // (silent mode aborts on locked files). A generated .cmd script run
      // by cmd.exe waits for this process to exit, runs the installer with
      // /S, relaunches the same executable path and logs every step to
      // %TEMP%\nexus-update.log - replacing the old PowerShell
      // helper, which never even started on some machines.
      const exe = process.execPath
      const logPath = path.join(os.tmpdir(), "nexus-update.log")
      const scriptPath = path.join(tmp, "nexus-update.cmd")
      const script = buildInstalledUpdateCmd({ exe, installer, tmpDir: tmp, pid: process.pid })
      fs.writeFileSync(scriptPath, script, "utf8")
      emit({ phase: "install", received: size, total: size, percent: 100 })
      spawnCmdHelper(scriptPath, logPath)
      emit({ phase: "relaunch", received: 0, total: null, percent: null })
      app.quit()
      // v0.13.10: same failsafe as the portable flavor - never let a
      // blocked quit sequence leave the app hanging after an update.
      setTimeout(() => app.exit(0), 2500)
      return { ok: true, error: null }
    }

    if (process.platform === "win32") {
      // v0.13.8: portable flavor. The archive is extracted HERE, inside
      // the running app (fflate streaming unzip) - extraction errors now
      // show up as normal in-app errors instead of silently dying after
      // the quit. Only the file swap must happen after this process exits
      // (the exe/dlls/asar stay locked while the app lives), and that is
      // done by a generated .cmd script executed by cmd.exe: move the old
      // exe aside, robocopy the extracted payload over the app directory,
      // verify the new exe, relaunch, roll back on failure. Built only
      // from Windows built-ins - no PowerShell, no execution policy, no
      // shell profile, nothing for AV/ASR rules to block.
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
      const exe = process.execPath
      const exeName = path.basename(exe)
      const appDir = path.dirname(exe)
      const extractDir = path.join(tmp, "extracted")
      fs.mkdirSync(extractDir, { recursive: true })
      emit({ phase: "install", received: 0, total: size, percent: 0 })
      await extractZip(zipPath, extractDir, (p) => {
        emit({ phase: "install", received: p.received, total: p.total, percent: Math.floor((p.received / p.total) * 100) })
      })
      // The portable zip stores files at the root; keep the nested-layout
      // fallback in case the packaging ever changes.
      const srcDir = locatePayloadExe(extractDir, exeName)
      if (!srcDir) {
        return { ok: false, error: "Downloaded archive does not contain the app executable - aborting update" }
      }
      emit({ phase: "install", received: size, total: size, percent: 100 })
      // v0.13.10: the post-exit swap is done by the NEW executable itself
      // (spawned with --apply-update below), not by a generated .cmd script -
      // on some machines AV/ASR rules killed the cmd.exe helper before it
      // could run a single line, leaving the app closed, files untouched
      // and nothing relaunched.
      const helperExe = path.join(srcDir, exeName)
      const logPath = path.join(os.tmpdir(), "nexus-update.log")
      try {
        fs.appendFileSync(
          logPath,
          new Date().toISOString() + " update ready, spawning apply helper " + helperExe + "\n"
        )
      } catch {
        /* logging must never break the update */
      }
      try {
        spawn(
          helperExe,
          [
            "--apply-update",
            "--pid",
            String(process.pid),
            "--src",
            srcDir,
            "--appdir",
            appDir,
            "--exe",
            exeName,
            "--tmpd",
            tmp,
          ],
          {
            detached: true,
            stdio: "ignore",
          }
        )
          .on("error", (e) => {
            try {
              fs.appendFileSync(logPath, new Date().toISOString() + " helper spawn failed: " + String(e) + "\n")
            } catch {
              /* nothing else to do - we are quitting */
            }
          })
          .unref()
      } catch (e) {
        try {
          fs.appendFileSync(logPath, new Date().toISOString() + " helper spawn threw: " + String(e) + "\n")
        } catch {
          /* nothing else to do - we are quitting */
        }
      }
      // The archive is already extracted; free ~230 MB right away.
      try {
        fs.rmSync(zipPath, { force: true })
      } catch {
        /* best effort */
      }
      emit({ phase: "relaunch", received: 0, total: null, percent: null })
      app.quit()
      // Failsafe: if anything blocks the normal quit sequence, hard-exit
      // anyway - the apply helper force-kills this pid if it lingers.
      setTimeout(() => app.exit(0), 2500)
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
