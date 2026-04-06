import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'

export interface VibeSessionStats {
  steps: number
  sessionPromptTokens: number
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

export function loadVibeSessions(): VibeSession[] {
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
        sessions.push(session)
      }
    } catch {
      // skip malformed
    }
  }

  return sessions.sort(
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
