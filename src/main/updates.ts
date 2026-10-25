import { execFile, spawn } from "child_process"
import https from "https"
import os from "os"
import path from "path"
import fs from "fs"
import { app } from "electron"
import { findBinary, tryExec } from "./setup"
import { sshRun } from "./remote"
import type { RemoteConnection } from "./config"

// v0.9.0: update system for the app itself (GitHub releases) and the
// mistral-vibe CLI (PyPI). The renderer orchestrates session restore after a
// CLI upgrade; this module only reports versions and performs the upgrades.

const GITHUB_REPO = "Dragonk/mistral-vibe-nexus"
const PYPI_PACKAGE = "mistral-vibe"

export interface AppUpdateInfo {
  currentVersion: string
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

export async function checkAppUpdate(): Promise<AppUpdateInfo> {
  const currentVersion = app.getVersion()
  try {
    const rel = (await fetchJson("https://api.github.com/repos/" + GITHUB_REPO + "/releases/latest")) as {
      tag_name?: string
      html_url?: string
      assets?: { name: string; browser_download_url: string }[]
    }
    const latestVersion = rel.tag_name ? rel.tag_name.replace(/^v/, "") : null
    const assets = rel.assets ?? []
    let downloadUrl: string | null = null
    if (process.platform === "win32") {
      const asset = assets.find((a) => a.name.toLowerCase().endsWith(".exe") && a.name.toLowerCase().includes("setup"))
      downloadUrl = asset?.browser_download_url ?? null
    } else if (process.platform === "linux") {
      const asset = assets.find((a) => a.name.toLowerCase().endsWith(".zip") && a.name.toLowerCase().includes("linux"))
      downloadUrl = asset?.browser_download_url ?? null
    }
    return {
      currentVersion,
      latestVersion,
      updateAvailable: latestVersion ? isNewerVersion(latestVersion, currentVersion) : false,
      releaseUrl: rel.html_url ?? null,
      downloadUrl,
      error: null,
    }
  } catch (err) {
    return {
      currentVersion,
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
  const vibe = findBinary("vibe")
  if (vibe) {
    const out = tryExec('"' + vibe + '" --version')
    if (out) return extractVersion(out)
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
      updateAvailable: current ? isNewerVersion(latest, current) : true,
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
      updateAvailable: current ? isNewerVersion(latest, current) : true,
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

function downloadTo(url: string, dest: string, redirects = 0): Promise<void> {
  return new Promise((resolve, reject) => {
    if (redirects > 5) {
      reject(new Error("Too many redirects while downloading"))
      return
    }
    const req = https.get(url, { headers: { "User-Agent": "mistral-vibe-nexus-updater" }, timeout: 120000 }, (res) => {
      if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume()
        const next = new URL(res.headers.location, url).toString()
        void downloadTo(next, dest, redirects + 1).then(resolve, reject)
        return
      }
      if (res.statusCode !== 200) {
        res.resume()
        reject(new Error("Download failed: HTTP " + res.statusCode))
        return
      }
      const f = fs.createWriteStream(dest)
      res.pipe(f)
      f.on("finish", () => {
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

export async function updateApp(): Promise<AppUpdateResult> {
  if (!app.isPackaged) {
    return { ok: false, error: "Self-update is only available for installed builds" }
  }
  const info = await checkAppUpdate()
  if (!info.downloadUrl) {
    return { ok: false, error: info.error ?? "No update asset available for this platform" }
  }
  const tmp = path.join(os.tmpdir(), "nexus-update-" + Date.now())
  fs.mkdirSync(tmp, { recursive: true })
  try {
    if (process.platform === "win32") {
      const installer = path.join(tmp, "nexus-setup.exe")
      await downloadTo(info.downloadUrl, installer)
      // The silent NSIS installer closes the running app itself; a detached
      // PowerShell helper waits for the install to finish and relaunches the
      // same executable path, so the app comes back with its persisted state
      // (config.json keeps sessions, serve settings and credentials).
      const exe = process.execPath
      const ps =
        "Start-Process -FilePath '" + installer + "' -ArgumentList '/S' -Wait; Start-Process -FilePath '" + exe + "'"
      spawn(
        "powershell.exe",
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps],
        { detached: true, stdio: "ignore", windowsHide: true }
      ).unref()
      setTimeout(() => {
        app.quit()
      }, 800)
      return { ok: true, error: null }
    }
    if (process.platform === "linux") {
      const zipPath = path.join(tmp, "nexus-linux.zip")
      await downloadTo(info.downloadUrl, zipPath)
      // User-dir layout (scripts/install-linux.sh): the app directory is the
      // folder holding the executable. Replacing files of a running Linux
      // process is safe - the kernel keeps the old inode mapped.
      const installDir = path.dirname(process.execPath)
      const r = await runAsync("unzip", ["-o", zipPath, "-d", installDir], 180000)
      if (r.code !== 0) {
        return { ok: false, error: "unzip failed: " + (r.stderr || r.stdout).trim().slice(0, 500) }
      }
      app.relaunch()
      app.exit(0)
      return { ok: true, error: null }
    }
    return { ok: false, error: "Unsupported platform for self-update: " + process.platform }
  } catch (err) {
    return { ok: false, error: (err as Error).message }
  }
}

