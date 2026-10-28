import fs from "fs"
import path from "path"
import { app } from "electron"

// v1.0.24: the prompt queue lives on the SERVER (the main process), not in
// any client. A prompt queued while a turn runs is delivered when the turn
// ends - even when EVERY client is closed in the meantime; the queue also
// survives an app restart (persisted in userData). Every connected client
// receives `queue:changed` broadcasts, so the queue view is identical
// everywhere (PC, second browser, phone PWA).

export interface QueuedPrompt {
  id: string
  acpSessionId: string
  remoteId: string | null
  /** cwd of the session - needed to revive/restore it on delivery */
  cwd: string
  /** text delivered to the agent (the @-expanded form) */
  text: string
  /** what the user message bubble shows (raw text) */
  displayText: string
  userMsg?: { id: string; text: string; attachments?: Array<{ path: string; isDir: boolean }> }
  images?: Array<{ data: string; mimeType: string; uri?: string }>
  queuedAt: number
}

let queue: QueuedPrompt[] = []

function queueFile(): string {
  return path.join(app.getPath("userData"), "prompt-queue.json")
}

export function loadQueue(): void {
  try {
    const raw = JSON.parse(fs.readFileSync(queueFile(), "utf-8")) as QueuedPrompt[]
    queue = Array.isArray(raw) ? raw.filter((q) => q && q.id && q.acpSessionId) : []
  } catch {
    queue = []
  }
}

function persist(): void {
  try {
    fs.writeFileSync(queueFile(), JSON.stringify(queue, null, 1))
  } catch {
    /* best effort */
  }
}

export function listQueue(): QueuedPrompt[] {
  return [...queue]
}

export function peekNextFor(acpSessionId: string): QueuedPrompt | null {
  return queue.find((q) => q.acpSessionId === acpSessionId) ?? null
}

export function addToQueue(item: QueuedPrompt): void {
  // one queued prompt per client-command id (idempotent retries)
  queue = queue.filter((q) => q.id !== item.id)
  queue.push(item)
  persist()
}

export function removeFromQueue(id: string): void {
  queue = queue.filter((q) => q.id !== id)
  persist()
}

export function updateQueueText(id: string, text: string, displayText: string): void {
  queue = queue.map((q) =>
    q.id === id ? { ...q, text, displayText, userMsg: q.userMsg ? { ...q.userMsg, text: displayText } : undefined } : q,
  )
  persist()
}

export function clearQueue(): void {
  queue = []
  persist()
}

export function queueSessionIds(): string[] {
  return Array.from(new Set(queue.map((q) => q.acpSessionId)))
}
