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
      proc = spawn(tokens[0], [...tokens.slice(1), remoteCmd], { windowsHide: true })
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

/** List sessions on the remote host by speaking JSON-RPC to vibe-acp over ssh. */
export function listRemoteSessions(conn: RemoteConnection): Promise<RemoteSessionInfo[]> {
  return new Promise((resolve) => {
    const tokens = tokenizeCmd(conn.sshCmd)
    if (tokens.length === 0) {
      resolve([])
      return
    }
    let proc
    try {
      // Login shell so the remote PATH includes ~/.local/bin (uv tools).
    proc = spawn(tokens[0], [...tokens.slice(1), 'bash -lc "exec vibe-acp"'], { windowsHide: true })
    } catch {
      resolve([])
      return
    }
    const sessions: RemoteSessionInfo[] = []
    let initId = ""
    let listId = ""
    let done = false
    const finish = (): void => {
      if (done) return
      done = true
      clearTimeout(timer)
      try { proc.kill() } catch { /* noop */ }
      resolve(sessions)
    }
    const timer = setTimeout(finish, 25000)
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
      const result = (msg.result ?? {}) as Record<string, unknown>
      if (id === initId) {
        listId = randomUUID()
        proc.stdin?.write(JSON.stringify({ jsonrpc: "2.0", id: listId, method: "session/list", params: {} }) + "\n")
      } else if (id === listId) {
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
        finish()
      }
    })
    proc.on("error", finish)
    proc.on("exit", finish)
    initId = randomUUID()
    proc.stdin?.write(
      JSON.stringify({
        jsonrpc: "2.0",
        id: initId,
        method: "initialize",
        params: { protocolVersion: 1, clientInfo: { name: "Mistral Vibe Nexus", version: "0.6.0" } },
      }) + "\n",
    )
    setTimeout(() => {
      if (!done && listId === "") {
        listId = randomUUID()
        try {
          proc.stdin?.write(JSON.stringify({ jsonrpc: "2.0", id: listId, method: "session/list", params: {} }) + "\n")
        } catch { finish() }
      }
    }, 15000)
  })
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
