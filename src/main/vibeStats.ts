import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'
import { loadVibeConfig, VibeModelConfig } from './vibeConfig'

export interface VibeSessionStats {
  steps: number
  sessionPromptTokens: number
  sessionCachedTokens?: number
  sessionCompletionTokens: number
  toolCallsAgreed: number
  toolCallsRejected: number
  toolCallsFailed: number
  toolCallsSucceeded: number
  contextTokens: number
  lastTurnDuration: number
  tokensPerSecond: number
  sessionTotalLlmTokens: number
  sessionCost: number
}

export interface VibeSession {
  sessionId: string
  startTime: string
  endTime: string
  title: string | null
  totalMessages: number
  gitBranch: string | null
  environment: { workingDirectory: string }
  stats: VibeSessionStats
  config: { activeModel: string } | null
  agentProfile: { name: string } | null
  models?: string[]
}

function camelToSnakeKeys(obj: unknown): unknown {
  if (Array.isArray(obj)) return obj.map(camelToSnakeKeys)
  if (obj && typeof obj === 'object') {
    const result: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      // Convert snake_case keys from JSON to camelCase
      const camel = k.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())
      result[camel] = camelToSnakeKeys(v)
    }
    return result
  }
  return obj
}

// ---------------------------------------------------------------------------
// Cost computation from config.toml model prices
// ---------------------------------------------------------------------------

interface ModelPrice {
  input: number
  output: number
  cached: number
}

function modelPriceFor(modelName: string | null | undefined, models: VibeModelConfig[]): ModelPrice | null {
  if (!modelName || models.length === 0) return null
  const lower = modelName.toLowerCase()
  const model = models.find(
    (m) =>
      m.alias === modelName ||
      m.name === modelName ||
      m.name.toLowerCase() === lower ||
      (m.alias ?? '').toLowerCase() === lower ||
      m.name.toLowerCase().endsWith(lower) ||
      lower.endsWith(m.name.toLowerCase())
  )
  if (!model) return null
  const input = typeof model.input_price === 'number' ? model.input_price : 0
  const output = typeof model.output_price === 'number' ? model.output_price : 0
  // cached_input_price null/undefined => model without cache support:
  // cached tokens are billed at the plain input price.
  const cached =
    typeof model.cached_input_price === 'number' && model.cached_input_price !== null
      ? model.cached_input_price
      : input
  return { input, output, cached }
}

function computeSessionCost(
  promptTokens: number,
  cachedTokens: number,
  completionTokens: number,
  price: ModelPrice
): number {
  const uncached = Math.max(0, promptTokens - cachedTokens)
  return (
    (uncached * price.input + cachedTokens * price.cached + completionTokens * price.output) /
    1_000_000
  )
}

function readJsonFile(filePath: string): Record<string, unknown> | null {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf-8')) as Record<string, unknown>
  } catch {
    return null
  }
}

/** Legacy depth-1 session dirs (~/.vibe/logs/session/<id>/meta.json). */
function loadLegacySessions(models: VibeModelConfig[]): VibeSession[] {
  const sessionDir = path.join(os.homedir(), '.vibe', 'logs', 'session')

  if (!fs.existsSync(sessionDir)) return []

  const sessions: VibeSession[] = []

  let entries: string[]
  try {
    entries = fs.readdirSync(sessionDir)
  } catch {
    return []
  }

  for (const entry of entries) {
    const metaPath = path.join(sessionDir, entry, 'meta.json')
    if (!fs.existsSync(metaPath)) continue
    try {
      const raw = fs.readFileSync(metaPath, 'utf-8')
      const parsed = JSON.parse(raw)
      const session = camelToSnakeKeys(parsed) as VibeSession
      if (session.sessionId && session.startTime) {
        // Recompute cost when the legacy log carries no pricing info
        if (session.stats && (session.stats.sessionCost ?? 0) === 0) {
          const price = modelPriceFor(session.config?.activeModel ?? null, models)
          if (price && (session.stats.sessionPromptTokens ?? 0) > 0) {
            session.stats.sessionCost = computeSessionCost(
              session.stats.sessionPromptTokens ?? 0,
              session.stats.sessionCachedTokens ?? 0,
              session.stats.sessionCompletionTokens ?? 0,
              price
            )
          }
        }
        sessions.push(session)
      }
    } catch {
      // skip malformed
    }
  }

  return sessions
}

// ---------------------------------------------------------------------------
// Unified session store (~/.vibe/logs/session/unified/<id>/) — new CLI format.
// meta.json has no token stats; usage lives in
// generations/<gen>/projection-state.json → snapshot.session.tokenUsage.
// ---------------------------------------------------------------------------

function latestGeneration(sessionDir: string): string | null {
  const current = readJsonFile(path.join(sessionDir, 'CURRENT'))
  if (current && typeof current.generation === 'string') return current.generation
  try {
    const gens = fs
      .readdirSync(path.join(sessionDir, 'generations'))
      .filter((e) => /^\d+$/.test(e))
      .sort()
    return gens.length > 0 ? gens[gens.length - 1] : null
  } catch {
    return null
  }
}

interface UnifiedUsage {
  inputTokens: number
  cachedInputTokens: number
  outputTokens: number
  totalTokens: number
  contextTokens: number
  toolCalls: number
  messageCount: number
}

function loadUnifiedSessionUsage(sessionDir: string): UnifiedUsage | null {
  const gen = latestGeneration(sessionDir)
  if (!gen) return null
  const ps = readJsonFile(path.join(sessionDir, 'generations', gen, 'projection-state.json'))
  if (!ps) return null
  const snapshot = (ps.snapshot ?? {}) as Record<string, unknown>
  const session = (snapshot.session ?? {}) as Record<string, unknown>
  const usage = (session.tokenUsage ?? {}) as Record<string, number>
  const context = (session.contextUsage ?? {}) as Record<string, number>
  const inputTokens = usage.inputTokens ?? 0
  const cachedInputTokens = usage.cachedInputTokens ?? 0
  const outputTokens = usage.outputTokens ?? 0
  const totalTokens = usage.totalTokens ?? inputTokens + outputTokens

  let toolCalls = 0
  try {
    toolCalls = fs.readdirSync(path.join(sessionDir, 'tool-results')).length
  } catch {
    toolCalls = 0
  }
  let messageCount = 0
  try {
    messageCount = fs.readdirSync(path.join(sessionDir, 'chunks')).length
  } catch {
    messageCount = 0
  }

  return {
    inputTokens,
    cachedInputTokens,
    outputTokens,
    totalTokens,
    contextTokens: context.totalTokens ?? inputTokens + outputTokens,
    toolCalls,
    messageCount,
  }
}

/** All models used across generations of a unified session (runtime-state.json). */
function loadUnifiedSessionModels(sessionDir: string, fallback: string | null): string[] {
  const used: string[] = []
  const generationsDir = path.join(sessionDir, 'generations')
  let gens: string[]
  try {
    gens = fs.readdirSync(generationsDir)
  } catch {
    return fallback ? [fallback] : []
  }
  for (const gen of gens.sort()) {
    const state = readJsonFile(path.join(generationsDir, gen, 'runtime-state.json'))
    const meta = (state?.['session_metadata'] ?? null) as Record<string, unknown> | null
    const model = (meta?.['active_model'] as string) ?? null
    if (model && !used.includes(model)) used.push(model)
  }
  if (used.length === 0 && fallback) used.push(fallback)
  return used
}

/** Last model actually used by a session (newest generation that has one). */
export function loadSessionLastModel(sessionId: string): string | null {
  const generationsDir = path.join(
    os.homedir(), '.vibe', 'logs', 'session', 'unified', sessionId, 'generations'
  )
  let gens: string[]
  try {
    gens = fs.readdirSync(generationsDir)
  } catch {
    return null
  }
  for (const gen of gens.sort().reverse()) {
    const state = readJsonFile(path.join(generationsDir, gen, 'runtime-state.json'))
    const meta = (state?.['session_metadata'] ?? null) as Record<string, unknown> | null
    const model = (meta?.['active_model'] as string) ?? null
    if (model) return model
  }
  return null
}

function loadUnifiedSessions(models: VibeModelConfig[]): VibeSession[] {
  const unifiedDir = path.join(os.homedir(), '.vibe', 'logs', 'session', 'unified')
  if (!fs.existsSync(unifiedDir)) return []

  let entries: string[]
  try {
    entries = fs.readdirSync(unifiedDir)
  } catch {
    return []
  }

  const sessions: VibeSession[] = []
  for (const entry of entries) {
    const sessionDir = path.join(unifiedDir, entry)
    try {
      if (!fs.statSync(sessionDir).isDirectory()) continue
    } catch {
      continue
    }
    const meta = readJsonFile(path.join(sessionDir, 'meta.json'))
    if (!meta) continue
    const sessionId = (meta.session_id ?? entry) as string
    const startTime = meta.start_time as string | undefined
    if (!startTime) continue

    const config = (meta.config ?? {}) as Record<string, unknown>
    const environment = (meta.environment ?? {}) as Record<string, unknown>
    const usage = loadUnifiedSessionUsage(sessionDir)
    const activeModel = (config.active_model as string) ?? null
    const price = modelPriceFor(activeModel, models)
    const inputTokens = usage?.inputTokens ?? 0
    const cachedTokens = usage?.cachedInputTokens ?? 0
    const outputTokens = usage?.outputTokens ?? 0

    sessions.push({
      sessionId,
      startTime,
      endTime: (meta.end_time as string) ?? startTime,
      title: (meta.title as string) ?? null,
      totalMessages: usage?.messageCount ?? 0,
      gitBranch: (meta.git_branch as string) ?? null,
      environment: { workingDirectory: (environment.working_directory as string) ?? '' },
      agentProfile: null,
      models: loadUnifiedSessionModels(sessionDir, activeModel),
      config: activeModel ? { activeModel } : null,
      stats: {
        steps: usage?.toolCalls ?? 0,
        sessionPromptTokens: inputTokens,
        sessionCachedTokens: cachedTokens,
        sessionCompletionTokens: outputTokens,
        toolCallsAgreed: usage?.toolCalls ?? 0,
        toolCallsRejected: 0,
        toolCallsFailed: 0,
        toolCallsSucceeded: usage?.toolCalls ?? 0,
        contextTokens: usage?.contextTokens ?? 0,
        lastTurnDuration: 0,
        tokensPerSecond: 0,
        sessionTotalLlmTokens: usage?.totalTokens ?? inputTokens + outputTokens,
        sessionCost: price
          ? computeSessionCost(inputTokens, cachedTokens, outputTokens, price)
          : 0,
      },
    })
  }

  return sessions
}

export function loadVibeSessions(): VibeSession[] {
  const models = (loadVibeConfig().models ?? []) as VibeModelConfig[]
  const legacy = loadLegacySessions(models)
  const unified = loadUnifiedSessions(models)

  // Unified store wins on duplicate session ids (same session, newer format)
  const byId = new Map<string, VibeSession>()
  for (const s of legacy) byId.set(s.sessionId, s)
  for (const s of unified) byId.set(s.sessionId, s)

  return Array.from(byId.values()).sort(
    (a, b) => new Date(b.startTime).getTime() - new Date(a.startTime).getTime()
  )
}

export interface VibeToolCall {
  id: string
  index: number
  function: { name: string; arguments: string }
  type: 'function'
}

export interface VibeToolResult {
  name: string
  tool_call_id: string
  content: string
}

export type VibeMessage = {
  role: 'user' | 'assistant' | 'tool' | 'system'
  content: string | null
  injected: boolean
  message_id: string
  tool_calls?: VibeToolCall[]
  name?: string
  tool_call_id?: string
}

export function loadVibeSessionMessages(sessionId: string): VibeMessage[] {
  const sessionDir = path.join(os.homedir(), '.vibe', 'logs', 'session')

  // Find the session folder by matching sessionId (could be in folder name or meta.json)
  let messagesPath: string | null = null

  let entries: string[]
  try {
    entries = fs.readdirSync(sessionDir)
  } catch {
    return []
  }

  for (const entry of entries) {
    const metaPath = path.join(sessionDir, entry, 'meta.json')
    if (!fs.existsSync(metaPath)) continue
    try {
      const raw = fs.readFileSync(metaPath, 'utf-8')
      const parsed = JSON.parse(raw)
      if (parsed.session_id === sessionId || entry.includes(sessionId)) {
        messagesPath = path.join(sessionDir, entry, 'messages.jsonl')
        break
      }
    } catch {
      // skip malformed
    }
  }

  if (!messagesPath || !fs.existsSync(messagesPath)) return []

  const messages: VibeMessage[] = []
  const fileContent = fs.readFileSync(messagesPath, 'utf-8')
  const lines = fileContent.split('\n').filter(line => line.trim())

  for (const line of lines) {
    try {
      const parsed = JSON.parse(line) as VibeMessage
      // Tool response messages (role:'tool') have tool_call_id but no message_id
      if (parsed.role && (parsed.message_id || parsed.tool_call_id)) {
        messages.push(parsed)
      }
    } catch {
      // skip malformed lines
    }
  }

  return messages
}

export interface VibeSubagentSession {
  sessionId: string
  agentName: string
  title: string | null
  startTime: string
  messages: VibeMessage[]
}

export function loadVibeSubagentSessions(parentSessionId: string): VibeSubagentSession[] {
  const sessionDir = path.join(os.homedir(), '.vibe', 'logs', 'session')

  let topEntries: string[]
  try {
    topEntries = fs.readdirSync(sessionDir)
  } catch {
    return []
  }

  // Find the parent session folder
  let parentFolder: string | null = null
  for (const entry of topEntries) {
    const metaPath = path.join(sessionDir, entry, 'meta.json')
    if (!fs.existsSync(metaPath)) continue
    try {
      const raw = fs.readFileSync(metaPath, 'utf-8')
      const parsed = JSON.parse(raw)
      if (parsed.session_id === parentSessionId || entry.includes(parentSessionId)) {
        parentFolder = path.join(sessionDir, entry)
        break
      }
    } catch { /* skip */ }
  }

  if (!parentFolder) return []

  const agentsDir = path.join(parentFolder, 'agents')
  if (!fs.existsSync(agentsDir)) return []

  const subSessions: VibeSubagentSession[] = []
  let agentEntries: string[]
  try {
    agentEntries = fs.readdirSync(agentsDir)
  } catch {
    return []
  }

  for (const entry of agentEntries) {
    const metaPath = path.join(agentsDir, entry, 'meta.json')
    const messagesPath = path.join(agentsDir, entry, 'messages.jsonl')
    if (!fs.existsSync(metaPath)) continue
    try {
      const raw = fs.readFileSync(metaPath, 'utf-8')
      const parsed = JSON.parse(raw)
      const messages: VibeMessage[] = []
      if (fs.existsSync(messagesPath)) {
        const lines = fs.readFileSync(messagesPath, 'utf-8').split('\n').filter(l => l.trim())
        for (const line of lines) {
          try {
            const m = JSON.parse(line) as VibeMessage
            if (m.role && m.message_id) messages.push(m)
          } catch { /* skip */ }
        }
      }
      subSessions.push({
        sessionId: parsed.session_id ?? entry,
        agentName: parsed.agent_profile?.name ?? entry.split('_')[0],
        title: parsed.title ?? null,
        startTime: parsed.start_time ?? '',
        messages,
      })
    } catch { /* skip */ }
  }

  return subSessions.sort((a, b) => new Date(a.startTime).getTime() - new Date(b.startTime).getTime())
}

export function loadVibeSessionMetadata(sessionId: string): { activeModel: string | null; agentProfile: string | null } {
  const sessionDir = path.join(os.homedir(), '.vibe', 'logs', 'session')

  let entries: string[]
  try {
    entries = fs.readdirSync(sessionDir)
  } catch {
    return { activeModel: null, agentProfile: null }
  }

  for (const entry of entries) {
    const metaPath = path.join(sessionDir, entry, 'meta.json')
    if (!fs.existsSync(metaPath)) continue
    try {
      const raw = fs.readFileSync(metaPath, 'utf-8')
      const parsed = JSON.parse(raw)
      if (parsed.session_id === sessionId || entry.includes(sessionId)) {
        return {
          activeModel: parsed.config?.active_model ?? null,
          agentProfile: parsed.agent_profile?.name ?? null
        }
      }
    } catch {
      // skip malformed
    }
  }

  return { activeModel: null, agentProfile: null }
}


// ---------------------------------------------------------------------------
// History content search: scan unified chunks + legacy messages.jsonl
// ---------------------------------------------------------------------------

export interface ContentMatch {
  sessionId: string
  snippet: string
}

export function searchSessionContent(query: string): ContentMatch[] {
  const q = query.toLowerCase()
  if (!q) return []
  const results: ContentMatch[] = []
  const seen = new Set<string>()
  const sessionRoot = path.join(os.homedir(), '.vibe', 'logs', 'session')

  const pushMatch = (sessionId: string, text: string): void => {
    if (seen.has(sessionId)) return
    const idx = text.toLowerCase().indexOf(q)
    if (idx === -1) return
    seen.add(sessionId)
    const from = Math.max(0, idx - 70)
    const raw = text.slice(from, idx + q.length + 70)
    results.push({ sessionId, snippet: raw.replace(/\s+/g, ' ').slice(0, 200) })
  }

  // Unified store: ~/.vibe/logs/session/unified/<id>/chunks/*
  const unifiedDir = path.join(sessionRoot, 'unified')
  try {
    const entries = fs.readdirSync(unifiedDir)
    for (const entry of entries) {
      const chunksDir = path.join(unifiedDir, entry, 'chunks')
      let files: string[]
      try { files = fs.readdirSync(chunksDir) } catch { continue }
      for (const f of files) {
        let text: string
        try { text = fs.readFileSync(path.join(chunksDir, f), 'utf-8') } catch { continue }
        pushMatch(entry, text)
        if (seen.has(entry)) break
      }
    }
  } catch { /* unified dir missing */ }

  // Legacy store: ~/.vibe/logs/session/<entry>/messages.jsonl
  try {
    const entries = fs.readdirSync(sessionRoot)
    for (const entry of entries) {
      if (entry === 'unified') continue
      const msgPath = path.join(sessionRoot, entry, 'messages.jsonl')
      if (!fs.existsSync(msgPath)) continue
      let text: string
      try { text = fs.readFileSync(msgPath, 'utf-8') } catch { continue }
      let sessionId = entry
      try {
        const meta = JSON.parse(fs.readFileSync(path.join(sessionRoot, entry, 'meta.json'), 'utf-8'))
        if (meta.session_id) sessionId = meta.session_id
      } catch { /* fall back to dir name */ }
      pushMatch(sessionId, text)
    }
  } catch { /* session root missing */ }

  return results
}

/** Last config actually used by a session: model, agent (mode), reasoning effort, cwd. */
export function loadSessionLastConfig(sessionId: string): {
  model: string | null;
  mode: string | null;
  thinking: string | null;
  cwd: string | null;
} | null {
  const generationsDir = path.join(
    os.homedir(), '.vibe', 'logs', 'session', 'unified', sessionId, 'generations'
  );
  let gens: string[];
  try {
    gens = fs.readdirSync(generationsDir);
  } catch {
    return null;
  }
  for (const gen of gens.sort().reverse()) {
    const state = readJsonFile(path.join(generationsDir, gen, 'runtime-state.json'));
    const meta = (state?.['session_metadata'] ?? null) as Record<string, unknown> | null;
    if (!meta) continue;
    const model = (meta['active_model'] as string) ?? null;
    if (!model) continue;
    return {
      model,
      mode: (meta['agent_name'] as string) ?? null,
      thinking: (meta['reasoning_effort'] as string) ?? null,
      cwd: (meta['cwd'] as string) ?? null,
    };
  }
  return null;
}

// ---------------------------------------------------------------------------
// v0.9.0: lazy session preview - reads the unified session store projection
// chunks straight from disk so the sidebar can show a session's history
// WITHOUT starting vibe-acp (real resume happens on the first message).
// ---------------------------------------------------------------------------

export interface PreviewToolCallDiff {
  path: string
  oldText: string | null
  newText: string
  startLine?: number | null
  newStartLine?: number | null
}

export interface SessionPreview {
  messages: Array<{
    role: 'user' | 'ai'
    content: string
    thinking?: Array<{ id: string; content: string }>
    toolCalls?: Array<{
      id: string
      title: string
      kind: string
      status: 'done' | 'error'
      input?: Record<string, unknown>
      output?: string
      diffs?: PreviewToolCallDiff[]
      previousContent?: string | null
      toolName?: string | null
      rawOutput?: Record<string, unknown> | null
      subagent?: { agent: string | null; agentName?: string | null; task: string; childSessionId: string | null } | null
    }>
  }>
  title: string | null
  cwd: string | null
  updatedAt: number | null
  config: { model: string | null; mode: string | null; thinking: string | null } | null
  usage: {
    contextUsed: number
    promptTokens: number
    completionTokens: number
    cachedTokens: number
    totalTokens: number
  } | null
  /** v0.11.1: last todo snapshot (session plan) from vibe.todo effects; null = no todos ever. */
  plan: Array<{
    content: string
    status: 'pending' | 'in_progress' | 'completed'
    priority?: string
  }> | null
}

interface PreviewRawEntry {
  [key: string]: unknown
}

const PREVIEW_MAX_CHUNKS = 48
const PREVIEW_MAX_MESSAGES = 2000
const PREVIEW_OUTPUT_CAP = 20000

/** Convert projection chunk entries into a UI-friendly history. */
export function buildSessionPreview(
  entries: PreviewRawEntry[],
  indexEntry: Record<string, unknown> | null,
): SessionPreview {
  const messages: SessionPreview['messages'] = []
  let plan: SessionPreview['plan'] = null
  let curAi: SessionPreview['messages'][number] | null = null
  let pendingThinking: Array<{ id: string; content: string }> = []
  const flushAi = (): void => {
    if (curAi) messages.push(curAi)
    curAi = null
  }
  for (const e of entries) {
    if (messages.length > PREVIEW_MAX_MESSAGES) break
    const type = e['type']
    if (type === 'message') {
      const role = e['role'] === 'user' ? 'user' : 'ai'
      const blocks = Array.isArray(e['content']) ? (e['content'] as Array<Record<string, unknown>>) : []
      const text = blocks
        .filter((b) => b && b['type'] === 'text')
        .map((b) => String(b['text'] ?? ''))
        .join('')
      if (role === 'user') {
        flushAi()
        pendingThinking = []
        messages.push({ role: 'user', content: text })
      } else {
        flushAi()
        curAi = {
          role: 'ai',
          content: text,
          thinking: pendingThinking.length > 0 ? pendingThinking : undefined,
        }
        pendingThinking = []
      }
    } else if (type === 'reasoning') {
      const block = { id: String(e['id'] ?? 'reasoning'), content: String(e['text'] ?? '') }
      if (curAi) {
        curAi.thinking = [...(curAi.thinking ?? []), block]
      } else {
        pendingThinking = [...pendingThinking, block]
      }
    } else if (type === 'effect') {
      if (!curAi) {
        curAi = {
          role: 'ai',
          content: '',
          thinking: pendingThinking.length > 0 ? pendingThinking : undefined,
        }
        pendingThinking = []
      }
      const det = (e['detail'] as Record<string, unknown> | undefined) ?? {}
      const display = (det['display'] as Record<string, unknown> | undefined) ?? {}
      const state = (e['state'] as Record<string, unknown> | undefined) ?? {}
      const out = (state['output'] as Record<string, unknown> | undefined) ?? {}
      const outBlocks = Array.isArray(out['content']) ? (out['content'] as Array<Record<string, unknown>>) : []
      let output = outBlocks
        .filter((b) => b && b['type'] === 'text')
        .map((b) => String(b['text'] ?? ''))
        .join('')
      if (output.length > PREVIEW_OUTPUT_CAP) output = output.slice(0, PREVIEW_OUTPUT_CAP) + '...'
      // v0.11.1: vibe.todo effects carry the session plan - keep the last snapshot
      if (String(det['toolName'] ?? '') === 'vibe.todo') {
        const tin = (det['input'] as Record<string, unknown> | undefined) ?? {}
        let todos = Array.isArray(tin['todos']) ? (tin['todos'] as Array<Record<string, unknown>>) : null
        if (!todos) {
          const sc = (out['structured_content'] as Record<string, unknown> | undefined) ?? {}
          todos = Array.isArray(sc['todos']) ? (sc['todos'] as Array<Record<string, unknown>>) : null
        }
        if (todos) {
          const entries: NonNullable<SessionPreview['plan']> = []
          for (const t of todos) {
            const status = String(t['status'] ?? 'pending')
            if (status === 'cancelled') continue
            entries.push({
              content: String(t['content'] ?? ''),
              status: status === 'completed' || status === 'in_progress' ? status : 'pending',
              priority: t['priority'] != null ? String(t['priority']) : undefined,
            })
          }
          plan = entries
        }
      }
      const decision = state['decision']
      const success =
        display['success'] !== false && (decision === undefined || decision === null || decision === 'execute')
      // v0.12: git-style edit diffs with line numbers (edit / search_replace).
      const sc = (out['structured_content'] as Record<string, unknown> | undefined) ?? {}
      const tin = (det['input'] as Record<string, unknown> | undefined) ?? {}
      const inPath = String(tin['filePath'] ?? tin['file_path'] ?? tin['path'] ?? '')
      // v0.13: subagent invocation (agents.spawn / task) - prompt + child session id.
      let psub: { agent: string | null; agentName?: string | null; task: string; childSessionId: string | null } | null = null
      if (String(det['kind'] ?? '') === 'subagent') {
        const saMetaAll = (out['_meta'] as Record<string, unknown> | undefined) ?? {}
        const saMeta = (saMetaAll['mistral.vibe.subagent'] as Record<string, unknown> | undefined) ?? undefined
        const childRaw = det['childSessionId'] ?? (saMeta ? saMeta['childSessionId'] : undefined)
        psub = {
          agent: typeof tin['agent'] === 'string' ? tin['agent'] : null,
          agentName: typeof det['agentName'] === 'string' ? det['agentName'] : null,
          task: typeof tin['task'] === 'string' ? tin['task'] : '',
          childSessionId: typeof childRaw === 'string' && childRaw.length > 0 ? childRaw : null,
        }
      }
      let pdiffs: PreviewToolCallDiff[] | undefined
      let prevContent: string | null = null
      const metaAll = (out['_meta'] as Record<string, unknown> | undefined) ?? {}
      const srMeta = (metaAll['mistralai.vibe.sdk.search_replace'] as Record<string, unknown> | undefined) ?? undefined
      if (srMeta && Array.isArray(srMeta['blocks'])) {
        const blocks = srMeta['blocks'] as Array<Record<string, unknown>>
        if (typeof srMeta['previous_content'] === 'string') prevContent = srMeta['previous_content']
        pdiffs = blocks.map((b) => ({
          path: String(sc['file'] ?? inPath),
          oldText: Array.isArray(b['old_lines']) ? (b['old_lines'] as unknown[]).map(String).join(String.fromCharCode(10)) : null,
          newText: Array.isArray(b['new_lines']) ? (b['new_lines'] as unknown[]).map(String).join(String.fromCharCode(10)) : '',
          startLine: typeof b['old_start_line'] === 'number' ? b['old_start_line'] : null,
          newStartLine: typeof b['new_start_line'] === 'number' ? b['new_start_line'] : null,
        }))
      } else if (Array.isArray(sc['occurrences'])) {
        const occ = sc['occurrences'] as Array<Record<string, unknown>>
        const pcv = out['previousContent'] ?? sc['previousContent']
        if (typeof pcv === 'string') prevContent = pcv
        pdiffs = occ.map((o) => ({
          path: String(o['file'] ?? sc['file'] ?? inPath),
          oldText: typeof o['oldText'] === 'string' ? o['oldText'] : null,
          newText: typeof o['newText'] === 'string' ? o['newText'] : '',
          startLine: typeof o['startLine'] === 'number' ? o['startLine'] : null,
          newStartLine: typeof o['newStartLine'] === 'number' ? o['newStartLine'] : null,
        }))
      } else {
        const pairs = (
          Array.isArray(tin['changes']) ? tin['changes'] : Array.isArray(tin['content']) ? tin['content'] : null
        ) as Array<Record<string, unknown>> | null
        if (pairs && inPath) {
          pdiffs = pairs.map((c) => ({
            path: inPath,
            oldText: c['oldString'] != null || c['old_str'] != null || c['old_string'] != null ? String(c['oldString'] ?? c['old_str'] ?? c['old_string']) : null,
            newText: c['newString'] != null || c['new_str'] != null || c['new_string'] != null ? String(c['newString'] ?? c['new_str'] ?? c['new_string']) : '',
          }))
        }
      }
      curAi.toolCalls = [
        ...(curAi.toolCalls ?? []),
        {
          id: String(e['id'] ?? 'effect'),
          title: String(display['summary'] ?? det['toolName'] ?? e['title'] ?? 'tool'),
          kind: 'tool',
          status: success ? 'done' : 'error',
          input: det['input'] as Record<string, unknown> | undefined,
          output: output.length > 0 ? output : undefined,
          diffs: pdiffs && pdiffs.length > 0 ? pdiffs : undefined,
          previousContent: prevContent,
          toolName: det['toolName'] != null ? String(det['toolName']) : null,
          rawOutput: String(det['toolName'] ?? '').startsWith('process.') ? out : undefined,
          subagent: psub,
        },
      ]
    }
  }
  flushAi()

  let title: string | null = null
  let cwd: string | null = null
  let updatedAt: number | null = null
  let config: SessionPreview['config'] = null
  let usage: SessionPreview['usage'] = null
  if (indexEntry) {
    const sess = (indexEntry['session'] as Record<string, unknown> | undefined) ?? {}
    const meta = (indexEntry['metadata'] as Record<string, unknown> | undefined) ?? {}
    title = typeof sess['title'] === 'string' ? sess['title'] : null
    cwd = typeof meta['cwd'] === 'string' ? meta['cwd'] : null
    updatedAt = typeof sess['updatedAt'] === 'number' ? sess['updatedAt'] : null
    config = {
      model: typeof meta['active_model'] === 'string' ? meta['active_model'] : null,
      mode: typeof meta['agent_name'] === 'string' ? meta['agent_name'] : null,
      thinking: typeof meta['reasoning_effort'] === 'string' ? meta['reasoning_effort'] : null,
    }
    const ctx = (sess['contextUsage'] as Record<string, unknown> | undefined) ?? {}
    const tok = (sess['tokenUsage'] as Record<string, unknown> | undefined) ?? {}
    const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0)
    if (ctx['totalTokens'] !== undefined || tok['totalTokens'] !== undefined) {
      usage = {
        contextUsed: num(ctx['totalTokens']),
        promptTokens: num(tok['inputTokens']),
        completionTokens: num(tok['outputTokens']),
        cachedTokens: num(tok['cachedInputTokens']),
        totalTokens: num(tok['totalTokens']),
      }
    }
  }
  return { messages, title, cwd, updatedAt, config, usage, plan }
}

function readSessionIndexEntry(sessionId: string): Record<string, unknown> | null {
  try {
    const p = path.join(os.homedir(), '.vibe', 'logs', 'session', 'unified', '.session-index.json')
    const idx = JSON.parse(fs.readFileSync(p, 'utf-8')) as Record<string, unknown>
    const entries = Array.isArray(idx['entries']) ? (idx['entries'] as Array<Record<string, unknown>>) : []
    for (const e of entries) {
      const ident = (e['identity'] as Record<string, unknown> | undefined) ?? {}
      if (ident['session_id'] === sessionId || e['session_id'] === sessionId) return e
    }
  } catch {}
  return null
}

function readUnifiedChunks(sessionDir: string): PreviewRawEntry[] | null {
  try {
    const cur = JSON.parse(fs.readFileSync(path.join(sessionDir, 'CURRENT'), 'utf-8')) as Record<string, unknown>
    const gen = String(cur['generation'] ?? '')
    const manifest = JSON.parse(
      fs.readFileSync(path.join(sessionDir, 'generations', gen, 'manifest.json'), 'utf-8'),
    ) as Record<string, unknown>
    const ps = (manifest['projection_state'] as Record<string, unknown> | undefined) ?? {}
    const shasAll = Array.isArray(ps['chunks']) ? (ps['chunks'] as string[]) : []
    const shas = shasAll.length > PREVIEW_MAX_CHUNKS ? shasAll.slice(shasAll.length - PREVIEW_MAX_CHUNKS) : shasAll
    const entries: PreviewRawEntry[] = []
    for (const sha of shas) {
      try {
        const p = path.join(sessionDir, 'chunks', sha + '.json')
        const data = JSON.parse(fs.readFileSync(p, 'utf-8')) as unknown
        if (Array.isArray(data)) entries.push(...(data as PreviewRawEntry[]))
      } catch {}
    }
    return entries
  } catch {
    return null
  }
}

/** Locate the unified session directory for an ACP session id. */
function findUnifiedSessionDir(sessionId: string): string | null {
  const base = path.join(os.homedir(), '.vibe', 'logs', 'session', 'unified')
  const direct = path.join(base, sessionId)
  if (fs.existsSync(path.join(direct, 'CURRENT'))) return direct
  try {
    for (const entry of fs.readdirSync(base)) {
      const cand = path.join(base, entry)
      try {
        if (!fs.statSync(cand).isDirectory()) continue
      } catch {
        continue
      }
      if (!fs.existsSync(path.join(cand, 'CURRENT'))) continue
      try {
        const meta = JSON.parse(fs.readFileSync(path.join(cand, 'meta.json'), 'utf-8')) as Record<string, unknown>
        if (meta['session_id'] === sessionId) return cand
      } catch {}
    }
  } catch {}
  return null
}

export function loadSessionPreview(sessionId: string): SessionPreview | null {
  const dir = findUnifiedSessionDir(sessionId)
  if (!dir) return null
  const entries = readUnifiedChunks(dir)
  if (!entries) return null
  return buildSessionPreview(entries, readSessionIndexEntry(sessionId))
}
