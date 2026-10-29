/**
 * v1.0.37: authoritative live transcript per ACP session.
 *
 * Why this exists
 * ---------------
 * The renderer used to be the only place where a running turn was assembled:
 * it appended `MessageChunk` text to whatever bubble its local
 * `streamingState` pointed at, guessed the session id when an event arrived
 * for an unknown one (`local?.id ?? currentSessionId`), and repaired itself by
 * REPLACING the whole transcript from the session file on every reconnect -
 * which deleted the live tail and duplicated text that the file and the
 * stream both delivered.
 *
 * Now the main process assembles the turn once, with STABLE message ids and a
 * monotonically increasing per-session `seq`:
 *
 *   - every event carries `msgId`, `seq` and (for text) `offset`/`delta`, so
 *     the client can apply it idempotently: a duplicate or reordered delta is
 *     detected by the offset instead of silently corrupting the bubble,
 *   - `snapshot()` returns the full transcript + `seq`, so a client that
 *     missed events (frozen tab, dropped websocket, app restart) can catch up
 *     without touching the disk file and without losing the live tail,
 *   - the disk replay (`session/load`) is fed into the same registry, so after
 *     a resume the ids and the shape match the live stream exactly.
 *
 * Assembly rules (deliberately simple and deterministic):
 *   - one AI segment = one message id, closed by a user message, by reasoning
 *     that arrives after streamed text, or by the end of the turn,
 *   - text/thinking are appended as deltas (a resent full text is reduced to
 *     its tail exactly once - no 40-character heuristic),
 *   - a tool call always attaches to the segment that was current when it
 *     started, never to "the last AI message" by index.
 */

export interface RegThinkingBlock {
  id: string
  content: string
}

export interface RegToolCall {
  id: string
  title: string
  kind: string
  status: 'running' | 'done' | 'error'
  input?: Record<string, unknown>
  output?: string
  diffs?: unknown
  previousContent?: string | null
  toolName?: string | null
  rawOutput?: Record<string, unknown> | null
  subagent?: Record<string, unknown> | null
}

export interface RegMessage {
  /** stable id shared by the live stream, snapshots and the client store */
  id: string
  role: 'user' | 'ai'
  content: string
  thinking: RegThinkingBlock[]
  toolCalls: RegToolCall[]
  /** the turn that produced this message has ended */
  complete: boolean
}

export interface RegPlanEntry {
  content: string
  status: 'pending' | 'in_progress' | 'completed'
  priority?: string
}

export interface SessionSnapshot {
  sessionId: string
  /** per-session monotonic counter; the ONLY ordering key on the client */
  seq: number
  messages: RegMessage[]
  busy: boolean
  title?: string
  plan?: RegPlanEntry[]
  updatedAt: number
}

interface Sent {
  /** text already emitted for this source id */
  text: string
}

interface SessionState {
  sessionId: string
  seq: number
  messages: RegMessage[]
  currentAiId: string | null
  /** last assembled message per ACP messageId (segment tracking) */
  segments: Map<string, string>
  sent: Map<string, Sent>
  segCounter: number
  busy: boolean
  title?: string
  plan?: RegPlanEntry[]
  updatedAt: number
}

const MAX_MESSAGES = 4000
const states = new Map<string, SessionState>()

function stateFor(sessionId: string): SessionState {
  let st = states.get(sessionId)
  if (!st) {
    st = {
      sessionId,
      seq: 0,
      messages: [],
      currentAiId: null,
      segments: new Map(),
      sent: new Map(),
      segCounter: 0,
      busy: false,
      updatedAt: Date.now(),
    }
    states.set(sessionId, st)
  }
  return st
}

function touch(st: SessionState): number {
  st.seq += 1
  st.updatedAt = Date.now()
  if (st.messages.length > MAX_MESSAGES) {
    st.messages.splice(0, st.messages.length - MAX_MESSAGES)
  }
  return st.seq
}

function newSegment(st: SessionState, key: string): RegMessage {
  st.segCounter += 1
  const msg: RegMessage = {
    id: 'ai-' + st.segCounter,
    role: 'ai',
    content: '',
    thinking: [],
    toolCalls: [],
    complete: false,
  }
  st.messages.push(msg)
  st.currentAiId = msg.id
  st.segments.set(key, msg.id)
  return msg
}

function segmentFor(st: SessionState, key: string): RegMessage {
  const id = st.segments.get(key)
  const found = id ? st.messages.find((m) => m.id === id) : undefined
  if (found) {
    st.currentAiId = found.id
    return found
  }
  return newSegment(st, key)
}

/**
 * Resend-safe delta. The CLI sometimes repeats the full accumulated text
 * (typically during a replay, but also after a subagent handoff); the old code
 * only stripped the prefix when the accumulated text was >= 40 characters,
 * which duplicated everything below that threshold. This version compares
 * lengths and prefixes exactly.
 */
function sliceDelta(st: SessionState, key: string, raw: string): string {
  const seen = st.sent.get(key)
  const prev = seen?.text ?? ''
  let delta = raw
  if (raw === prev) {
    delta = ''
  } else if (raw.length > prev.length && raw.startsWith(prev)) {
    delta = raw.slice(prev.length)
  }
  st.sent.set(key, { text: prev + delta })
  return delta
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

export function beginTurn(sessionId: string): void {
  const st = stateFor(sessionId)
  // a new turn starts a fresh segment for the same ACP messageId
  st.segments.clear()
  st.sent.clear()
  st.currentAiId = null
  st.busy = true
  touch(st)
}

export function endTurn(sessionId: string): number {
  const st = stateFor(sessionId)
  for (const msg of st.messages) {
    if (!msg.complete) msg.complete = true
  }
  st.busy = false
  st.currentAiId = null
  return touch(st)
}

export function markBusy(sessionId: string, busy: boolean): number {
  const st = stateFor(sessionId)
  st.busy = busy
  return touch(st)
}

export interface TextDelta {
  msgId: string
  offset: number
  delta: string
  seq: number
}

export interface ThinkingDelta extends TextDelta {
  blockId: string
}

/** Streamed answer text. `acpMessageId` groups a logical agent message. */
export function appendText(sessionId: string, acpMessageId: string | null, raw: string): TextDelta | null {
  if (!raw) return null
  const st = stateFor(sessionId)
  const key = 'm|' + (acpMessageId ?? 'seg' + st.segCounter)
  const msg = segmentFor(st, key)
  const delta = sliceDelta(st, key, raw)
  if (!delta) {
    return { msgId: msg.id, offset: msg.content.length, delta: '', seq: st.seq }
  }
  const offset = msg.content.length
  msg.content += delta
  msg.complete = false
  return { msgId: msg.id, offset, delta, seq: touch(st) }
}

/**
 * Reasoning. It opens a NEW segment when the current one already has streamed
 * text, so a thought block never glues itself under the answer it precedes.
 */
export function appendThinking(
  sessionId: string,
  blockId: string | null,
  raw: string,
): ThinkingDelta | null {
  if (!raw) return null
  const st = stateFor(sessionId)
  let msg = st.currentAiId ? st.messages.find((m) => m.id === st.currentAiId) : undefined
  if (!msg || msg.content.length > 0) {
    st.segCounter += 1
    msg = newSegment(st, 'th|' + (blockId ?? 'x') + '|' + st.segCounter)
  }
  const key = 't|' + (blockId ?? 'thinking')
  const delta = sliceDelta(st, key, raw)
  if (!delta) {
    const existing = msg.thinking.find((b) => b.id === (blockId ?? 'thinking'))
    return {
      msgId: msg.id,
      blockId: blockId ?? 'thinking',
      offset: existing?.content.length ?? 0,
      delta: '',
      seq: st.seq,
    }
  }
  const id = blockId ?? 'thinking'
  let block = msg.thinking.find((b) => b.id === id)
  if (!block) {
    block = { id, content: '' }
    msg.thinking.push(block)
  }
  const offset = block.content.length
  block.content += delta
  return { msgId: msg.id, blockId: id, offset, delta, seq: touch(st) }
}

/** A user prompt (optimistic echo from any client, or a queued delivery). */
export function noteUserMessage(
  sessionId: string,
  msgId: string,
  text: string,
): RegMessage {
  const st = stateFor(sessionId)
  const existing = st.messages.find((m) => m.id === msgId)
  if (existing) {
    existing.content = text
    touch(st)
    return existing
  }
  const msg: RegMessage = {
    id: msgId,
    role: 'user',
    content: text,
    thinking: [],
    toolCalls: [],
    complete: true,
  }
  st.messages.push(msg)
  // the answer always starts a new segment after a user message
  st.currentAiId = null
  st.segments.clear()
  st.sent.clear()
  touch(st)
  return msg
}

export function startToolCall(sessionId: string, toolCall: RegToolCall): { msgId: string; seq: number } {
  const st = stateFor(sessionId)
  const msg = segmentFor(st, 'tc|' + toolCall.id)
  msg.toolCalls.push(toolCall)
  return { msgId: msg.id, seq: touch(st) }
}

export function updateToolCall(
  sessionId: string,
  toolCallId: string,
  patch: Partial<RegToolCall>,
): { msgId: string | null; seq: number } {
  const st = stateFor(sessionId)
  for (const msg of st.messages) {
    const tc = msg.toolCalls.find((t) => t.id === toolCallId)
    if (tc) {
      Object.assign(tc, patch)
      return { msgId: msg.id, seq: touch(st) }
    }
  }
  return { msgId: null, seq: st.seq }
}

export function setPlan(sessionId: string, plan: RegPlanEntry[]): number {
  const st = stateFor(sessionId)
  st.plan = plan
  return touch(st)
}

export function setTitle(sessionId: string, title: string | undefined): number {
  const st = stateFor(sessionId)
  if (title) st.title = title
  return touch(st)
}

export function resetSession(sessionId: string): void {
  states.delete(sessionId)
}

/**
 * Seed the registry from a `session/load` replay so a resumed session has the
 * same shape (and ids) as a live one. The history comes from `assembleHistory`
 * which reads the CLI's own replay.
 */
export function loadHistory(
  sessionId: string,
  history: Array<{
    role: 'user' | 'ai'
    content: string
    thinking?: Array<{ id: string; content: string }>
    toolCalls?: RegToolCall[]
  }>,
  title?: string,
  plan?: RegPlanEntry[],
): void {
  const st = stateFor(sessionId)
  st.messages = []
  st.segments.clear()
  st.sent.clear()
  st.segCounter = 0
  st.currentAiId = null
  for (const h of history) {
    if (h.role === 'user') {
      st.messages.push({
        id: 'u-' + st.messages.length,
        role: 'user',
        content: h.content,
        thinking: [],
        toolCalls: [],
        complete: true,
      })
      continue
    }
    st.segCounter += 1
    st.messages.push({
      id: 'ai-' + st.segCounter,
      role: 'ai',
      content: h.content,
      thinking: (h.thinking ?? []).map((b) => ({ id: b.id, content: b.content })),
      toolCalls: h.toolCalls ?? [],
      complete: true,
    })
  }
  if (title) st.title = title
  if (plan) st.plan = plan
  st.updatedAt = Date.now()
  st.seq += 1
}

export function snapshot(sessionId: string): SessionSnapshot {
  const st = stateFor(sessionId)
  return {
    sessionId,
    seq: st.seq,
    busy: st.busy,
    title: st.title,
    plan: st.plan,
    updatedAt: st.updatedAt,
    messages: st.messages.map((m) => ({
      id: m.id,
      role: m.role,
      content: m.content,
      thinking: m.thinking.map((b) => ({ ...b })),
      toolCalls: m.toolCalls.map((t) => ({ ...t })),
      complete: m.complete,
    })),
  }
}

export function hasSession(sessionId: string): boolean {
  const st = states.get(sessionId)
  return Boolean(st && st.messages.length > 0)
}

export function busySessions(): string[] {
  return [...states.values()].filter((s) => s.busy).map((s) => s.sessionId)
}
