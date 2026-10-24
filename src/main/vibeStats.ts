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
