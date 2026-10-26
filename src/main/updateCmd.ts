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
import { Unzip, UnzipInflate, UnzipPassThrough } from "fflate"

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

export interface PortableUpdateCmdParams {
  exe: string
  appDir: string
  oldName: string
  oldExe: string
  srcDir: string
  tmpDir: string
  pid: number
}

/**
 * v0.13.8: post-exit portable self-update script. Runs after the app has
 * quit: moves the old exe aside, robocopies the extracted payload over the
 * app directory, relaunches the new exe, and rolls back to the previous
 * exe on any failure. Robocopy exit codes 0-7 are success (with warnings),
 * 8+ are failures.
 */
export function buildPortableUpdateCmd(p: PortableUpdateCmdParams): string {
  const lines = cmdHeader()
  lines.push(
    'set "PID=' + p.pid + '"',
    'set "EXE=' + p.exe + '"',
    'set "APPDIR=' + p.appDir + '"',
    'set "OLDNAME=' + p.oldName + '"',
    'set "OLDEXE=' + p.oldExe + '"',
    'set "SRC=' + p.srcDir + '"',
    'set "TMPD=' + p.tmpDir + '"',
    "echo [%DATE% %TIME%] nexus portable update start (pid " + p.pid + ') >> "%LOG%"',
    ...cmdWaitForExit(p.pid),
    'echo [%TIME%] renaming old exe >> "%LOG%"',
    'ren "%EXE%" "%OLDNAME%" >nul 2>&1',
    "if errorlevel 1 goto renamefail",
    "goto renamed",
    ":renamefail",
    'move /y "%EXE%" "%OLDEXE%" >nul 2>&1',
    "if errorlevel 1 goto fatalrename",
    ":renamed",
    'echo [%TIME%] copying new files (robocopy) >> "%LOG%"',
    '%SystemRoot%\\System32\\robocopy.exe "%SRC%" "%APPDIR%" /E /R:3 /W:2 /NFL /NDL /NP /NJH /NJS >> "%LOG%" 2>&1',
    "set RCLVL=%ERRORLEVEL%",
    'echo [%TIME%] robocopy exit code %RCLVL% >> "%LOG%"',
    "if %RCLVL% GEQ 8 goto rollback",
    'if not exist "%EXE%" goto rollback',
    'echo [%TIME%] relaunching app >> "%LOG%"',
    'start "" "%EXE%"',
    "%SystemRoot%\\System32\\ping.exe -n 6 127.0.0.1 >nul",
    'del /f /q "%OLDEXE%" >nul 2>&1',
    'rd /s /q "%TMPD%" >nul 2>&1',
    'del /f /q "%FAIL%" >nul 2>&1',
    'echo [%TIME%] portable update done >> "%LOG%"',
    "exit /b 0",
    ":rollback",
    'echo update failed (robocopy exit %RCLVL% or new exe missing), rolled back to the previous version > "%FAIL%"',
    'move /y "%OLDEXE%" "%EXE%" >nul 2>&1',
    'start "" "%EXE%"',
    'echo [%TIME%] rolled back to previous exe >> "%LOG%"',
    "exit /b 1",
    ":fatalrename",
    "echo cannot rename the app executable (is the app directory locked by another process?) > \"%FAIL%\"",
    'if exist "%OLDEXE%" (start "" "%OLDEXE%") else (start "" "%EXE%")',
    'echo [%TIME%] could not rename old exe >> "%LOG%"',
    "exit /b 1",
  )
  return lines.join(CRLF)
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
