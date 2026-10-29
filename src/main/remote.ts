import { spawn } from "child_process"
import * as readline from "readline"
import { randomUUID } from "crypto"
import type { RemoteConnection } from "./config"

/** Split an ssh command string into argv tokens (respects quotes). */
export function tokenizeCmd(cmd: string): string[] {
  const tokens: string[] = []
  let cur = ""
  let quote: string | null = null
  for (const ch of cmd) {
    if (quote) {
      if (ch === quote) quote = null
      else cur += ch
    } else if (ch === '"' || ch === "'") {
      quote = ch
    } else if (/\s/.test(ch)) {
      if (cur) { tokens.push(cur); cur = "" }
    } else cur += ch
  }
  if (cur) tokens.push(cur)
  return tokens
}

/** Quote a string for the remote POSIX shell (single quotes, escape). */
export function shq(s: string): string {
  return "'" + s.replace(/'/g, "'\\''") + "'"
}

export interface SshRunResult { code: number; stdout: string; stderr: string }

/** Options every ssh spawn gets: accept unknown host keys (first connect
 * used to hang on the interactive prompt, blocking the session list for
 * minutes) and fail fast instead of hanging on an unreachable network. */
const SSH_CONNECT_OPTS = ["-o", "StrictHostKeyChecking=accept-new", "-o", "ConnectTimeout=10"]

/** Run a remote command over the connection's ssh command (single quoted arg). */
export function sshRun(
  conn: RemoteConnection,
  remoteCmd: string,
  timeoutMs = 15000,
  stdinData?: string,
): Promise<SshRunResult> {
  return new Promise((resolve) => {
    const tokens = tokenizeCmd(conn.sshCmd)
    if (tokens.length === 0) {
      resolve({ code: -1, stdout: "", stderr: "Empty ssh command" })
      return
    }
    let proc
    try {
      proc = spawn(tokens[0], [...tokens.slice(1), ...SSH_CONNECT_OPTS, remoteCmd], { windowsHide: true })
    } catch (err) {
      resolve({ code: -1, stdout: "", stderr: String(err) })
      return
    }
    let out = ""
    let err = ""
    let done = false
    const finish = (code: number): void => {
      if (done) return
      done = true
      clearTimeout(timer)
      resolve({ code, stdout: out, stderr: err })
    }
    const timer = setTimeout(() => {
      try { proc.kill() } catch { /* already gone */ }
      finish(-1)
    }, timeoutMs)
    proc.stdout?.on("data", (d: Buffer) => { out += d.toString() })
    proc.stderr?.on("data", (d: Buffer) => { err += d.toString() })
    if (stdinData !== undefined) {
      try {
        proc.stdin?.write(stdinData)
        proc.stdin?.end()
      } catch { /* stdin not writable */ }
    }
    proc.on("error", (e: Error) => { err += String(e); finish(-1) })
    proc.on("exit", (code) => finish(code ?? -1))
  })
}

export interface RemoteTestResult {
  reachable: boolean
  acpInstalled: boolean
  acpPath: string | null
  version: string | null
  error: string | null
}

/** Test an (unsaved) connection draft: reachability + vibe-acp presence. */
export async function testRemoteConnection(conn: RemoteConnection): Promise<RemoteTestResult> {
  const ping = await sshRun(conn, "echo __NEXUS_OK__", 12000)
  if (ping.code !== 0 || !ping.stdout.includes("__NEXUS_OK__")) {
    return {
      reachable: false,
      acpInstalled: false,
      acpPath: null,
      version: null,
      error: (ping.stderr.trim() || ping.stdout.trim()).slice(0, 300) || "SSH connection failed",
    }
  }
  // Login shell: non-interactive ssh sessions do not include ~/.local/bin in PATH.
  const which = await sshRun(conn, 'bash -lc "command -v vibe-acp"', 10000)
  const acpPath = which.code === 0 ? which.stdout.trim() : ""
  if (!acpPath) {
    return { reachable: true, acpInstalled: false, acpPath: null, version: null, error: null }
  }
  const ver = await sshRun(conn, "vibe-acp --version 2>/dev/null || true", 10000)
  return {
    reachable: true,
    acpInstalled: true,
    acpPath,
    version: ver.stdout.trim() || null,
    error: null,
  }
}

const DEFAULT_INSTALL_CMD =
  "sh -c 'if command -v uv >/dev/null 2>&1; then uv tool install mistral-vibe 2>/dev/null || uv tool upgrade mistral-vibe; else curl -LsSf https://astral.sh/uv/install.sh | sh && $HOME/.local/bin/uv tool install mistral-vibe; fi'"

export interface RemoteInstallResult { ok: boolean; error: string | null; log: string }

/** Install (or upgrade) vibe-acp on the remote host. */
export async function installRemoteAcp(conn: RemoteConnection): Promise<RemoteInstallResult> {
  const cmd = conn.installCommand?.trim() || DEFAULT_INSTALL_CMD
  const res = await sshRun(conn, cmd, 300000)
  const ok = res.code === 0
  return {
    ok,
    error: ok ? null : (res.stderr.trim() || "install failed").slice(0, 400),
    log: (res.stdout + res.stderr).slice(0, 2000),
  }
}

export interface RemoteSessionInfo {
  sessionId: string
  title: string
  cwd: string
  updatedAt: string
}

export interface RemoteSessionsResult {
  sessions: RemoteSessionInfo[]
  ok: boolean
  /** exit code + stderr when the handshake failed (surfaced in the UI) */
  error: string | null
}

/**
 * List sessions on the remote host by speaking JSON-RPC to vibe-acp over ssh.
 *
 * v1.0.37: failures are REPORTED instead of looking like an empty history.
 * The old version resolved `[]` for a refused ssh, a missing vibe-acp, a
 * timeout or a protocol error alike, so the phone showed an empty SSH group
 * with no explanation at all.
 */
export function listRemoteSessionsDetailed(conn: RemoteConnection): Promise<RemoteSessionsResult> {
  return new Promise((resolve) => {
    const tokens = tokenizeCmd(conn.sshCmd)
    if (tokens.length === 0) {
      resolve({ sessions: [], ok: false, error: "Empty ssh command" })
      return
    }
    let proc
    try {
      // Login shell so the remote PATH includes ~/.local/bin (uv tools).
      proc = spawn(tokens[0], [...tokens.slice(1), ...SSH_CONNECT_OPTS, 'bash -lc "exec vibe-acp"'], {
        windowsHide: true,
      })
    } catch (err) {
      resolve({ sessions: [], ok: false, error: String(err) })
      return
    }
    const sessions: RemoteSessionInfo[] = []
    let initId = ""
    let listId = ""
    let cursor = ""
    let pages = 0
    let stderr = ""
    let answeredList = false
    proc.stderr?.on("data", (d: Buffer) => {
      stderr += d.toString()
    })
    // The wire layer can paginate session/list (next_cursor); follow every
    // page so remote history is never truncated to the first page.
    const requestList = (): void => {
      listId = randomUUID()
      const params: Record<string, unknown> = {}
      if (cursor !== "") params["cursor"] = cursor
      proc.stdin?.write(
        JSON.stringify({ jsonrpc: "2.0", id: listId, method: "session/list", params }) + "\n",
      )
    }
    let done = false
    const finish = (ok: boolean, error: string | null): void => {
      if (done) return
      done = true
      clearTimeout(timer)
      const code = proc.exitCode
      try {
        proc.kill()
      } catch {
        /* noop */
      }
      const reason =
        error ??
        (ok
          ? null
          : "ssh exit " +
            String(code ?? "?") +
            (stderr.trim() ? ": " + stderr.trim().slice(0, 400) : " (no stderr)"))
      resolve({ sessions, ok: ok && answeredList, error: ok && answeredList ? null : reason })
    }
    const timer = setTimeout(() => finish(false, "session/list timed out after 25 s"), 25000)
    const rl = readline.createInterface({ input: proc.stdout! })
    rl.on("line", (line: string) => {
      let msg: Record<string, unknown>
      try {
        msg = JSON.parse(line) as Record<string, unknown>
      } catch {
        return
      }
      const id = msg.id as string | undefined
      if (!id) return
      const error = msg.error as { code?: number; message?: string } | undefined
      if (error) {
        finish(false, "vibe-acp error " + String(error.code ?? "?") + ": " + String(error.message ?? ""))
        return
      }
      const result = (msg.result ?? {}) as Record<string, unknown>
      if (id === initId) {
        requestList()
      } else if (id === listId) {
        answeredList = true
        const raw = (result.sessions ?? result.items ?? []) as Array<Record<string, unknown>>
        for (const s of raw) {
          const sessionId = (s.sessionId ?? s.id ?? "") as string
          if (!sessionId) continue
          sessions.push({
            sessionId,
            title: (s.title ?? s.name ?? "") as string,
            cwd: (s.cwd ?? s.workingDirectory ?? "") as string,
            updatedAt: (s.updatedAt ?? s.updated_at ?? s.startTime ?? "") as string,
          })
        }
        const nextCursor = (result.next_cursor ?? result.nextCursor ?? null) as string | null
        if (nextCursor && nextCursor !== "" && pages < 20) {
          pages += 1
          cursor = nextCursor
          requestList()
        } else {
          finish(true, null)
        }
      }
    })
    proc.on("error", (e: Error) => finish(false, "ssh spawn failed: " + String(e)))
    proc.on("exit", () => {
      // an exit before the list answer is a real failure, not an empty history
      if (!answeredList) finish(false, null)
      else finish(true, null)
    })
    initId = randomUUID()
    proc.stdin?.write(
      JSON.stringify({
        jsonrpc: "2.0",
        id: initId,
        method: "initialize",
        params: { protocolVersion: 1, clientInfo: { name: "Halny Vibe Deck", version: "1.0.37" } },
      }) + "\n",
    )
    setTimeout(() => {
      if (!done && listId === "") {
        try {
          requestList()
        } catch {
          finish(false, "cannot write to the ssh channel")
        }
      }
    }, 15000)
  })
}

/** Backwards compatible wrapper (desktop paths that only need the list). */
export async function listRemoteSessions(conn: RemoteConnection): Promise<RemoteSessionInfo[]> {
  const res = await listRemoteSessionsDetailed(conn)
  if (!res.ok && res.error) console.error("[remote] session list failed:", conn.name, res.error)
  return res.sessions
}

export interface RemoteStatus { id: string; ok: boolean; checkedAt: number; latencyMs: number | null }

/** Quick liveness check used for the green/red dot in the sidebar. */
export async function checkRemoteStatus(conn: RemoteConnection): Promise<RemoteStatus> {
  const t0 = Date.now()
  const res = await sshRun(conn, "true", 8000)
  return {
    id: conn.id,
    ok: res.code === 0,
    checkedAt: Date.now(),
    latencyMs: res.code === 0 ? Date.now() - t0 : null,
  }
}
