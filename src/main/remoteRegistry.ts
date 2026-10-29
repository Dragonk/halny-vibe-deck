import fs from "fs"
import path from "path"
import { app } from "electron"
import { broadcastToWeb } from "./bus"
import { loadConfig } from "./config"
import type { RemoteConnection } from "./config"
import { checkRemoteStatus, listRemoteSessionsDetailed } from "./remote"
import type { RemoteSessionInfo, RemoteStatus } from "./remote"

/**
 * v1.0.37: one server-side registry for SSH connections.
 *
 * Before this, every client (desktop window, phone PWA, second browser) ran
 * its own ssh round-trips in the renderer: a status probe per connection
 * followed by a 25 s `vibe-acp` handshake per connection, gated on the probe
 * succeeding. A phone that opened the app cold after a websocket blip got an
 * empty group with no error at all, and the double load (PC + phone) made
 * every ssh slower. Now the main process owns the state:
 *
 *  - single-flight per connection, one ssh per connection regardless of how
 *    many clients ask (TTL 30 s, longer on failure),
 *  - the last known session list is persisted, so a fresh client shows the
 *    SSH sessions immediately after login,
 *  - failures carry their reason (exit code + stderr) all the way to the UI
 *    instead of looking like "no sessions",
 *  - a change is broadcast once (`remote:updated`) instead of every client
 *    polling on its own.
 */

export interface RemoteEntryState {
  status: RemoteStatus | null
  sessions: RemoteSessionInfo[]
  /** human readable reason when the last refresh failed */
  error: string | null
  /** epoch ms of the last successful session fetch */
  fetchedAt: number
  /** a refresh is in flight right now */
  loading: boolean
}

interface Entry {
  status: RemoteStatus | null
  sessions: RemoteSessionInfo[]
  error: string | null
  fetchedAt: number
  inFlight: Promise<void> | null
  /** back off failed refreshes a little so a dead host is not hammered */
  retryAfter: number
}

const entries = new Map<string, Entry>()
const SESSION_TTL_MS = 30_000
const FAILURE_TTL_MS = 15_000

function cacheFile(): string {
  return path.join(app.getPath("userData"), "remote-cache.json")
}

interface CachedState {
  [connId: string]: { sessions: RemoteSessionInfo[]; fetchedAt: number }
}

/** Last known session lists survive an app restart. */
export function loadRemoteCache(): void {
  try {
    const raw = JSON.parse(fs.readFileSync(cacheFile(), "utf8")) as CachedState
    for (const [id, value] of Object.entries(raw ?? {})) {
      if (!value || !Array.isArray(value.sessions)) continue
      entries.set(id, {
        status: null,
        sessions: value.sessions,
        error: null,
        fetchedAt: value.fetchedAt ?? 0,
        inFlight: null,
        retryAfter: 0,
      })
    }
  } catch {
    /* no cache yet */
  }
}

function persist(): void {
  const out: CachedState = {}
  for (const [id, entry] of entries) {
    if (entry.sessions.length > 0) {
      out[id] = { sessions: entry.sessions, fetchedAt: entry.fetchedAt }
    }
  }
  try {
    fs.writeFileSync(cacheFile(), JSON.stringify(out), { mode: 0o600 })
  } catch {
    /* best effort */
  }
}

export function remoteState(): Record<string, RemoteEntryState> {
  const out: Record<string, RemoteEntryState> = {}
  for (const [id, entry] of entries) {
    out[id] = {
      status: entry.status,
      sessions: entry.sessions,
      error: entry.error,
      fetchedAt: entry.fetchedAt,
      loading: entry.inFlight !== null,
    }
  }
  return out
}

function broadcast(): void {
  broadcastToWeb("remote:updated", [remoteState()])
}

function entryFor(id: string): Entry {
  let entry = entries.get(id)
  if (!entry) {
    entry = { status: null, sessions: [], error: null, fetchedAt: 0, inFlight: null, retryAfter: 0 }
    entries.set(id, entry)
  }
  return entry
}

/**
 * Refresh one connection: liveness probe, then the session list. Both run
 * even when the probe fails - the list attempt carries the real error, and a
 * host that answers ssh but not `vibe-acp` must still show something.
 */
async function refreshOne(conn: RemoteConnection): Promise<void> {
  const entry = entryFor(conn.id)
  entry.error = null
  const status = await checkRemoteStatus(conn)
  entry.status = status
  const listed = await listRemoteSessionsDetailed(conn)
  if (listed.ok) {
    entry.sessions = listed.sessions
    entry.error = null
    entry.fetchedAt = Date.now()
    persist()
  } else {
    // keep the last known list; report why it is stale
    entry.error = listed.error ?? "ssh failed"
    entry.fetchedAt = entry.fetchedAt || 0
  }
  entry.retryAfter = listed.ok ? 0 : Date.now() + FAILURE_TTL_MS
}

function enabledConnections(ids?: string[]): RemoteConnection[] {
  return (loadConfig().remoteConnections ?? []).filter(
    (c) => c.enabled && (!ids || ids.length === 0 || ids.includes(c.id)),
  )
}

/**
 * Single-flight refresh. Concurrent callers share the in-flight promise, so a
 * phone and a desktop asking at the same time produce exactly one ssh.
 */
export async function refreshRemote(ids?: string[], force = false): Promise<Record<string, RemoteEntryState>> {
  const conns = enabledConnections(ids)
  const now = Date.now()
  const work: Array<Promise<void>> = []
  for (const conn of conns) {
    const entry = entryFor(conn.id)
    if (entry.inFlight) {
      work.push(entry.inFlight)
      continue
    }
    const fresh = entry.fetchedAt > 0 && now - entry.fetchedAt < SESSION_TTL_MS
    if (!force && fresh && !entry.error) continue
    if (!force && entry.retryAfter > now) continue
    entry.inFlight = refreshOne(conn).finally(() => {
      entry.inFlight = null
      broadcast()
    })
    work.push(entry.inFlight)
  }
  // a client waiting on refresh must never hang on one dead host
  await Promise.allSettled(work.map((p) => Promise.race([p, new Promise<void>((r) => setTimeout(r, 40_000))])))
  broadcast()
  return remoteState()
}

/** Drop connections that no longer exist in the config. */
export function pruneRemoteState(): void {
  const keep = new Set((loadConfig().remoteConnections ?? []).map((c) => c.id))
  let changed = false
  for (const id of [...entries.keys()]) {
    if (!keep.has(id)) {
      entries.delete(id)
      changed = true
    }
  }
  if (changed) {
    persist()
    broadcast()
  }
}

/** Background poll - only while somebody is actually watching. */
export function startRemotePoller(hasViewers: () => boolean): () => void {
  const timer = setInterval(() => {
    if (!hasViewers()) return
    void refreshRemote(undefined, false)
  }, 60_000)
  return () => clearInterval(timer)
}
