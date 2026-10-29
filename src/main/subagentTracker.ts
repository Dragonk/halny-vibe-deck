import { loadSessionPreview } from "./vibeStats"

/**
 * v1.0.37: live subagent tracking.
 *
 * A spawned subagent runs inside the CLI, in its own child session. The ACP
 * layer only reports the parent's `agents.spawn` tool call - and it keeps that
 * call `in_progress` for the child's whole life, while the app used to close it
 * on PromptComplete. The UI therefore showed a green tick on an agent that was
 * still working, and "show work" could only read the child transcript once
 * from disk (and, for a spawn card, never at all).
 *
 * The CLI does expose the child session on disk (`unified/child-*`), so this
 * tracker samples it while the child runs and pushes compact state updates:
 * elapsed time, the steps the child executed, and its transcript. Everything is
 * append-only and capped, so a long child cannot flood the websocket.
 */

export interface SubagentStep {
  title: string
  status: string
}

export interface SubagentWorkMessage {
  role: string
  content: string
  thinking?: Array<{ id: string; content: string }>
  toolCalls?: SubagentStep[]
}

export interface Run {
  toolCallId: string
  parentSessionId: string
  childSessionId: string
  agent: string | null
  task: string | null
  startedAt: number
  status: "running" | "done" | "error" | "unknown"
  signature: string
  truncated: boolean
  messages: SubagentWorkMessage[]
  steps: SubagentStep[]
  /** parent tool call reached a terminal status */
  parentSettled: boolean
  lastChangeAt: number
  lastEmitAt: number
  emit: (ev: { type: string; data: Record<string, unknown> }) => void
}

const runs = new Map<string, Run>()
const POLL_MS = 2000
/** a child that produced nothing for this long is reported as finished */
const IDLE_DONE_MS = 45_000
/** hard stop for the tracker, independent of the child */
const MAX_RUN_MS = 60 * 60 * 1000
const MAX_MESSAGES = 60
const MAX_MESSAGE_CHARS = 4000

let timer: ReturnType<typeof setInterval> | null = null

function ensureTimer(): void {
  if (timer) return
  timer = setInterval(poll, POLL_MS)
  // never keep the process alive just for this
  if (typeof timer.unref === "function") timer.unref()
}

function stopTimerIfIdle(): void {
  if (runs.size === 0 && timer) {
    clearInterval(timer)
    timer = null
  }
}

function signatureOf(messages: SubagentWorkMessage[]): string {
  const last = messages[messages.length - 1]
  const toolCount = messages.reduce((n, m) => n + (m.toolCalls?.length ?? 0), 0)
  return (
    messages.length +
    "|" +
    toolCount +
    "|" +
    (last ? last.role + ":" + last.content.length : "0")
  )
}

function cap(messages: SubagentWorkMessage[]): { messages: SubagentWorkMessage[]; truncated: boolean } {
  const truncated = messages.length > MAX_MESSAGES
  const slice = truncated ? messages.slice(-MAX_MESSAGES) : messages
  return {
    truncated,
    messages: slice.map((m) => ({
      role: m.role,
      content:
        m.content.length > MAX_MESSAGE_CHARS
          ? m.content.slice(0, MAX_MESSAGE_CHARS) + "…"
          : m.content,
      thinking: (m.thinking ?? []).map((b) => ({
        id: b.id,
        content: b.content.length > MAX_MESSAGE_CHARS ? b.content.slice(0, MAX_MESSAGE_CHARS) : b.content,
      })),
      toolCalls: (m.toolCalls ?? []).slice(-30),
    })),
  }
}

function collect(childSessionId: string): { messages: SubagentWorkMessage[]; steps: SubagentStep[]; done: boolean } {
  const preview = loadSessionPreview(childSessionId) as
    | {
        messages?: Array<{
          role: string
          content: string
          thinking?: Array<{ id: string; content: string }>
          toolCalls?: Array<{ title: string; status: string }>
        }>
        plan?: Array<{ content: string; status: string }> | null
      }
    | null
  const messages: SubagentWorkMessage[] = (preview?.messages ?? []).map((m) => ({
    role: m.role,
    content: m.content ?? "",
    thinking: m.thinking,
    toolCalls: (m.toolCalls ?? []).map((tc) => ({ title: tc.title, status: tc.status })),
  }))
  const plan = preview?.plan ?? []
  const steps: SubagentStep[] = plan.length
    ? plan.map((p) => ({ title: p.content, status: p.status }))
    : messages
        .flatMap((m) => m.toolCalls ?? [])
        .slice(-40)
        .map((tc) => ({ title: tc.title, status: tc.status }))
  const lastMessage = messages[messages.length - 1]
  // the child session file marks a closed turn with a complete last message
  const done = Boolean(lastMessage && lastMessage.role === "assistant" && messages.length > 0 && !lastMessage.toolCalls?.some((t) => t.status === "running"))
  return { messages, steps, done }
}

function publish(run: Run, force = false): void {
  const now = Date.now()
  if (!force && now - run.lastEmitAt < POLL_MS - 100) return
  run.lastEmitAt = now
  run.emit({
    type: run.status === "running" ? "SubagentProgress" : "SubagentFinished",
    data: {
      parentSessionId: run.parentSessionId,
      toolCallId: run.toolCallId,
      childSessionId: run.childSessionId,
      agent: run.agent,
      task: run.task,
      status: run.status,
      elapsedMs: now - run.startedAt,
      steps: run.steps,
      messages: run.messages,
      truncated: run.truncated,
    },
  })
}

function poll(): void {
  const now = Date.now()
  for (const run of [...runs.values()]) {
    if (run.status !== "running") {
      runs.delete(run.toolCallId)
      continue
    }
    let collected: ReturnType<typeof collect>
    try {
      collected = collect(run.childSessionId)
    } catch {
      collected = { messages: run.messages, steps: run.steps, done: false }
    }
    const capped = cap(collected.messages)
    const signature = signatureOf(capped.messages)
    if (signature !== run.signature) {
      run.signature = signature
      run.messages = capped.messages
      run.steps = collected.steps
      run.truncated = capped.truncated
      run.lastChangeAt = now
      publish(run, true)
    }
    const idle = now - run.lastChangeAt > IDLE_DONE_MS
    const overrun = now - run.startedAt > MAX_RUN_MS
    if ((collected.done && (idle || run.parentSettled)) || overrun) {
      run.status = run.parentSettled || collected.done ? "done" : "unknown"
      publish(run, true)
      runs.delete(run.toolCallId)
    }
  }
  stopTimerIfIdle()
}

/** Register a subagent spawn; polling starts immediately. */
export function trackSubagent(opts: {
  toolCallId: string
  parentSessionId: string
  childSessionId: string
  agent?: string | null
  task?: string | null
  emit: (ev: { type: string; data: Record<string, unknown> }) => void
}): void {
  if (!opts.childSessionId) return
  const now = Date.now()
  const run: Run = {
    toolCallId: opts.toolCallId,
    parentSessionId: opts.parentSessionId,
    childSessionId: opts.childSessionId,
    agent: opts.agent ?? null,
    task: opts.task ?? null,
    startedAt: now,
    status: "running",
    signature: "",
    truncated: false,
    messages: [],
    steps: [],
    parentSettled: false,
    lastChangeAt: now,
    lastEmitAt: 0,
    emit: opts.emit,
  }
  runs.set(opts.toolCallId, run)
  ensureTimer()
  // first sample right away so the card shows something within a tick
  try {
    const initial = collect(run.childSessionId)
    const capped = cap(initial.messages)
    run.messages = capped.messages
    run.steps = initial.steps
    run.signature = signatureOf(capped.messages)
  } catch {
    /* the child may not have written anything yet */
  }
  publish(run, true)
}

/** The parent's tool call reported a terminal status. */
export function noteToolStatus(toolCallId: string, status: "running" | "done" | "error"): void {
  const run = runs.get(toolCallId)
  if (!run) return
  if (status === "running") return
  run.parentSettled = true
  if (status === "error") {
    run.status = "error"
    publish(run, true)
    runs.delete(toolCallId)
    stopTimerIfIdle()
    return
  }
  // the child may still be finishing its last message - poll once more
  const collected = collect(run.childSessionId)
  const capped = cap(collected.messages)
  run.messages = capped.messages
  run.steps = collected.steps
  run.truncated = capped.truncated
  run.status = "done"
  publish(run, true)
  runs.delete(toolCallId)
  stopTimerIfIdle()
}

export function trackedRuns(): Array<{ toolCallId: string; parentSessionId: string; childSessionId: string }> {
  return [...runs.values()].map((r) => ({
    toolCallId: r.toolCallId,
    parentSessionId: r.parentSessionId,
    childSessionId: r.childSessionId,
  }))
}
