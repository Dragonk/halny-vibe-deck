// v0.13.8: Windows self-update helpers that no longer depend on PowerShell.
//
// The old helper was a generated .ps1 executed via powershell.exe -File. On
// some machines it never ran at all (AV/ASR rules blocking a PowerShell
// spawned from an app with a script generated in %TEMP%, constrained
// language mode, or the script dying before its first log line) - the app
// closed, no file changed, nothing relaunch. The new flow:
//
//   1. The running app extracts the downloaded archive itself with fflate
//      (streaming, constant memory), so extraction problems surface as
//      normal in-app errors while the user still has the app open.
//   2. Everything that must happen after the app exits (exe/dlls/asar stay
//      locked while it lives) is done by a generated .cmd script executed
//      by cmd.exe - built only from Windows built-ins (tasklist, taskkill,
//      ren, move, robocopy, start, ping, del, rd): no execution policy, no
//      shell profile, nothing for AV rules to single out.
//
// Both scripts append to %TEMP%\nexus-update.log. On failure they drop
// %TEMP%\nexus-update-failed.txt, which the app shows as a dialog on the
// next launch, so a failed update is never silent again.

import fs from "fs"
import path from "path"
import { execFile, spawn } from "child_process"
import { Unzip, UnzipInflate, UnzipPassThrough } from "fflate"
import os from "os"

export interface ExtractZipProgress {
  received: number
  total: number
}

/**
 * v0.13.8: stream a zip archive onto disk without loading it into memory.
 * Uses fflate's synchronous decoders, so by the time the final chunk has
 * been pushed every file is fully written.
 */
export function extractZip(zipPath: string, destDir: string, onProgress?: (p: ExtractZipProgress) => void): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false
    let fileCount = 0
    const fail = (err: Error): void => {
      if (!settled) {
        settled = true
        reject(err)
      }
    }
    const total = fs.statSync(zipPath).size
    let received = 0
    const uz = new Unzip()
    uz.register(UnzipInflate)
    uz.register(UnzipPassThrough)
    uz.onfile = (file) => {
      // entry names use forward slashes; directory entries end with "/"
      const raw = String(file.name)
      if (raw.endsWith("/")) return
      // refuse path traversal outside destDir
      const segments = raw.split("/").filter((s) => s.length > 0 && s !== "." && s !== "..")
      if (segments.length === 0) return
      const dest = path.join(destDir, ...segments)
      fs.mkdirSync(path.dirname(dest), { recursive: true })
      file.ondata = (err, chunk, final) => {
        if (err) {
          fail(new Error("extract " + raw + ": " + (err.message || String(err))))
          return
        }
        if (chunk && chunk.length > 0) {
          try {
            fs.appendFileSync(dest, chunk)
          } catch (e) {
            fail(new Error("write " + raw + ": " + (e as Error).message))
            return
          }
        }
        if (final) fileCount += 1
      }
      file.start()
    }
    const rs = fs.createReadStream(zipPath, { highWaterMark: 512 * 1024 })
    rs.on("data", (chunk: Buffer) => {
      if (settled) return
      try {
        uz.push(chunk)
      } catch (e) {
        rs.destroy()
        fail(new Error("corrupt archive: " + (e as Error).message))
        return
      }
      received += chunk.length
      try {
        onProgress?.({ received, total })
      } catch {
        /* progress reporting must never break the update */
      }
    })
    rs.on("end", () => {
      if (settled) return
      try {
        uz.push(new Uint8Array(0), true)
      } catch (e) {
        fail(new Error("corrupt archive: " + (e as Error).message))
        return
      }
      if (!settled) {
        settled = true
        if (fileCount === 0) reject(new Error("archive contains no files"))
        else resolve()
      }
    })
    rs.on("error", (e) => fail(e))
  })
}

/**
 * v0.13.8: find the directory that directly contains the app executable in
 * the extracted payload. The portable zip stores files at the root
 * (scripts/create-portable.js zips win-unpacked with archive.directory(...,
 * false)); the nested single-subfolder layout is kept as a fallback in case
 * the packaging ever changes.
 */
export function locatePayloadExe(extractDir: string, exeName: string): string | null {
  if (fs.existsSync(path.join(extractDir, exeName))) return extractDir
  try {
    for (const entry of fs.readdirSync(extractDir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue
      const sub = path.join(extractDir, entry.name)
      if (fs.existsSync(path.join(sub, exeName))) return sub
    }
  } catch {
    /* unreadable - treated as not found */
  }
  return null
}

const CRLF = "\r\n"

function cmdHeader(): string[] {
  return [
    // cmd parses batch files in the ANSI codepage; paths with diacritics
    // (user profile folders) would turn into mojibake. Switch to UTF-8
    // before any path is referenced - the file is written as UTF-8 without
    // a BOM so this first line itself stays pure ASCII.
    "@chcp 65001 >nul",
    "@echo off",
    "setlocal",
    "rem Generated by Mistral Vibe Nexus - safe to delete.",
    'set "LOG=%TEMP%\\nexus-update.log"',
    'set "FAIL=%TEMP%\\nexus-update-failed.txt"',
  ]
}

/**
 * Shared wait-for-parent section: poll tasklist for the app PID (up to
 * 120 s), force-kill on timeout, then give the OS a moment to release the
 * file locks before touching the app directory.
 */
function cmdWaitForExit(pid: number): string[] {
  return [
    "set /a TRIES=0",
    ":waitloop",
    '%SystemRoot%\\System32\\tasklist.exe /FI "PID eq ' + pid + '" /NH 2>nul | find.exe "' + pid + '" >nul',
    "if errorlevel 1 goto appdead",
    "set /a TRIES=TRIES+1",
    "if %TRIES% GEQ 120 goto forcekill",
    "%SystemRoot%\\System32\\ping.exe -n 2 127.0.0.1 >nul",
    "goto waitloop",
    ":forcekill",
    'echo [%TIME%] app still running after 120s, force killing >> "%LOG%"',
    "%SystemRoot%\\System32\\taskkill.exe /PID " + pid + " /F >nul 2>&1",
    ":appdead",
    "%SystemRoot%\\System32\\ping.exe -n 3 127.0.0.1 >nul",
  ]
}

export interface InstalledUpdateCmdParams {
  exe: string
  installer: string
  tmpDir: string
  pid: number
}

/**
 * v0.13.8: post-exit installed (NSIS) self-update script: waits for the app
 * to exit, runs the installer silently (/S - it aborts on locked files if
 * the app is still alive), relaunches the same exe path.
 */
export function buildInstalledUpdateCmd(p: InstalledUpdateCmdParams): string {
  const lines = cmdHeader()
  lines.push(
    'set "PID=' + p.pid + '"',
    'set "EXE=' + p.exe + '"',
    'set "INSTALLER=' + p.installer + '"',
    'set "TMPD=' + p.tmpDir + '"',
    "echo [%DATE% %TIME%] nexus installed update start (pid " + p.pid + ') >> "%LOG%"',
    ...cmdWaitForExit(p.pid),
    'echo [%TIME%] launching silent installer >> "%LOG%"',
    'start "" /wait "%INSTALLER%" /S',
    "set INSTLVL=%ERRORLEVEL%",
    'echo [%TIME%] installer exit code %INSTLVL% >> "%LOG%"',
    'echo [%TIME%] relaunching app >> "%LOG%"',
    'start "" "%EXE%"',
    "%SystemRoot%\\System32\\ping.exe -n 3 127.0.0.1 >nul",
    'rd /s /q "%TMPD%" >nul 2>&1',
    "if not %INSTLVL%==0 goto instfail",
    'del /f /q "%FAIL%" >nul 2>&1',
    'echo [%TIME%] installed update done >> "%LOG%"',
    "exit /b 0",
    ":instfail",
    "echo installer exited with code %INSTLVL% > \"%FAIL%\"",
    'echo [%TIME%] installer failed >> "%LOG%"',
    "exit /b 1",
  )
  return lines.join(CRLF)
}

// ---------------------------------------------------------------------------
// v0.13.10: portable self-update apply helper. The v0.13.8 flow spawned a
// generated .cmd script through cmd.exe; on some machines that helper never
// started at all (AV/ASR rules block script execution from the temp dir),
// leaving the app closed with the files untouched and nothing relaunched.
// The new approach has NO script and NO shell: the old app spawns the NEW
// executable itself with --apply-update right before it exits (see
// updates.ts / index.ts). The helper waits for the old process to die,
// copies the extracted payload over the app directory, relaunches the app
// and rolls back on failure. Every step is appended to nexus-update.log in
// the temp dir; a failure also drops nexus-update-failed.txt, which the app
// shows as a dialog on the next launch.
// ---------------------------------------------------------------------------

export function updateLogPath(): string {
  return path.join(os.tmpdir(), "nexus-update.log")
}

export function updateFailMarkerPath(): string {
  return path.join(os.tmpdir(), "nexus-update-failed.txt")
}

function logUpdate(line: string): void {
  try {
    fs.appendFileSync(updateLogPath(), new Date().toISOString() + " " + line + "\n")
  } catch {
    /* logging must never break the update */
  }
}

const sleepMs = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms))

function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (e) {
    // EPERM means the process exists but belongs to another user;
    // anything else (ESRCH) means it is gone.
    return (e as NodeJS.ErrnoException).code === "EPERM"
  }
}

function forceKillPid(pid: number): Promise<void> {
  return new Promise((resolve) => {
    if (process.platform === "win32") {
      const sysRoot = process.env.SystemRoot
      const taskkill = sysRoot ? path.join(sysRoot, "System32", "taskkill.exe") : "taskkill"
      try {
        execFile(taskkill, ["/PID", String(pid), "/F", "/T"], () => resolve())
      } catch {
        resolve()
      }
    } else {
      try {
        process.kill(pid, "SIGKILL")
      } catch {
        /* already gone */
      }
      resolve()
    }
  })
}

async function waitForPidDeath(pid: number, killAfterMs: number): Promise<void> {
  const start = Date.now()
  while (isPidAlive(pid)) {
    if (Date.now() - start >= killAfterMs) {
      logUpdate("old process " + pid + " still alive after " + killAfterMs + " ms, force killing")
      await forceKillPid(pid)
      const deadline = Date.now() + 10000
      while (isPidAlive(pid) && Date.now() < deadline) await sleepMs(250)
      return
    }
    await sleepMs(500)
  }
}

export interface PortableApplyParams {
  pid: number
  srcDir: string
  appDir: string
  exeName: string
  tmpDir: string
}

/**
 * v0.13.10: runs inside the NEW executable (--apply-update). Waits for the
 * old process to exit, swaps the files, relaunches the app. On failure the
 * previous executable is restored and a marker file is left behind.
 */
export async function applyPortableUpdate(p: PortableApplyParams, killAfterMs: number = 30000): Promise<void> {
  const t0 = Date.now()
  logUpdate("apply helper start (old pid " + p.pid + ", src " + p.srcDir + ")")
  await waitForPidDeath(p.pid, killAfterMs)
  logUpdate("old process gone, swapping files")
  const newExe = path.join(p.appDir, p.exeName)
  const oldExe = path.join(p.appDir, "nexus-old-" + Date.now() + ".exe")
  const hadOld = fs.existsSync(newExe)
  if (hadOld) {
    try {
      fs.renameSync(newExe, oldExe)
    } catch (e) {
      logUpdate("rename of the old exe failed (" + String(e) + "), killing again and retrying")
      await forceKillPid(p.pid)
      await sleepMs(1500)
      fs.renameSync(newExe, oldExe)
    }
  }
  try {
    fs.cpSync(p.srcDir, p.appDir, { recursive: true, force: true })
    if (!fs.existsSync(newExe)) throw new Error("new executable missing after copy")
    logUpdate("copy complete in " + (Date.now() - t0) + " ms")
  } catch (err) {
    logUpdate("copy failed: " + String(err))
    try {
      if (hadOld && fs.existsSync(newExe)) fs.rmSync(newExe, { force: true })
    } catch {
      /* best effort */
    }
    try {
      if (hadOld && fs.existsSync(oldExe)) fs.renameSync(oldExe, newExe)
    } catch {
      /* best effort */
    }
    try {
      fs.writeFileSync(
        updateFailMarkerPath(),
        "Portable update failed: " + String(err) + "\nThe previous version was restored."
      )
    } catch {
      /* best effort */
    }
    try {
      if (fs.existsSync(newExe)) {
        const child = spawn(newExe, [], { detached: true, stdio: "ignore" })
        child.on("error", () => {})
        child.unref()
      }
    } catch {
      /* best effort */
    }
    throw new Error("portable update copy failed: " + String(err))
  }
  try {
    if (fs.existsSync(oldExe)) fs.rmSync(oldExe, { force: true })
  } catch {
    /* best effort */
  }
  logUpdate("restarting app")
  try {
    const child = spawn(newExe, [], { detached: true, stdio: "ignore" })
    child.on("error", (e) => logUpdate("relaunch failed: " + String(e)))
    child.unref()
  } catch (e) {
    logUpdate("relaunch threw: " + String(e))
  }
  try {
    fs.rmSync(p.tmpDir, { recursive: true, force: true })
  } catch {
    /* the helper exe lives here while running - leftovers are swept on next start */
  }
  logUpdate("apply helper done in " + (Date.now() - t0) + " ms")
}

/**
 * v0.13.10: sweep leftover nexus-update-* directories (hundreds of MB of
 * payload) from previous updates. A finished update is at least a few
 * minutes old by the time a fresh instance calls this, and no update
 * continues across app starts, so the age threshold is safe.
 */
export function cleanupStaleUpdateDirs(maxAgeMs: number = 10 * 60 * 1000): void {
  try {
    const tmp = os.tmpdir()
    const now = Date.now()
    const entries = fs.readdirSync(tmp, { withFileTypes: true })
    for (const entry of entries) {
      if (!entry.isDirectory() || !entry.name.startsWith("nexus-update-")) continue
      const dir = path.join(tmp, entry.name)
      try {
        if (now - fs.statSync(dir).mtimeMs > maxAgeMs) fs.rmSync(dir, { recursive: true, force: true })
      } catch {
        /* locked or unreadable - keep it for the next sweep */
      }
    }
  } catch {
    /* never break startup over a cleanup */
  }
}
