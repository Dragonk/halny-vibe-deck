import type { RemoteConnection, RemoteStatus, RemoteSessionInfo, RemoteTestResult, ServeStatus } from '../../preload/index'

export type { RemoteConnection, RemoteStatus, RemoteSessionInfo, RemoteTestResult, ServeStatus }

export interface FileChange {
  path: string
  action: 'added' | 'modified' | 'deleted'
  additions?: number
  deletions?: number
}

export interface ToolCallDiff {
  path: string
  oldText: string | null
  newText: string
}

export interface ToolCall {
  id: string
  title: string
  kind: string
  status: 'running' | 'done' | 'error'
  input?: Record<string, unknown>
  output?: string
  fileChanges?: FileChange[]
  diffs?: ToolCallDiff[]
}

export interface ThinkingBlock {
  id: string
  content: string
  isComplete: boolean
  startedAt?: number
  completedAt?: number
}

export interface Attachment {
  path: string
  isDir: boolean
  content: string
}

export interface Message {
  id: string
  sender: 'user' | 'ai' | 'system'
  content: string
  isComplete: boolean
  timestamp: number
  toolCalls?: ToolCall[]
  thinking?: ThinkingBlock[]
  isContext?: boolean
  attachments?: Attachment[]
}

export interface Session {
  id: string
  name: string
  acpSessionId?: string
  messages: Message[]
  contextSummary?: string
  createdAt: number
  lastActive: number
  startTime?: number
  workingDirectory?: string
  updatedAt?: number
  loading?: boolean
  /** Model/mode/thinking last chosen by the user in this session (per-session isolation). */
  lastModel?: string
  lastMode?: string
  lastThinking?: string
  /** set when this session runs on a remote SSH connection */
  remoteId?: string
}

export type AppView = 'chat' | 'settings' | 'stats' | 'history'

export type AcpStatus = 'disconnected' | 'connecting' | 'connected' | 'error'

export type ThemeMode = 'auto' | 'dark' | 'light'

/** Dock main tabs (preview tabs are dynamic). */
export type DockTabKind = 'files' | 'git'

/** An open preview tab in the dock: a file or a git diff. */
export interface PreviewTab {
  id: string
  kind: 'file' | 'diff'
  name: string
  path: string
  diff?: string
  remoteId?: string | null
}

/** One selectable choice of an ACP config option. */
export interface ConfigOptionChoice {
  value: string
  name: string
  description?: string
}

/** A config option reported by the ACP agent (model, mode, thinking, ...). */
export interface ConfigOption {
  id: string
  name: string
  description?: string
  category?: string
  type: 'select' | 'boolean'
  currentValue: string | boolean
  options: ConfigOptionChoice[]
}

/** Live usage/telemetry for one session (from usage_update). */
export interface UsageStats {
  contextUsed: number
  contextSize: number
  cost?: number
  currency?: string
  steps?: number
  promptTokens?: number
  completionTokens?: number
  cachedTokens?: number
  totalTokens?: number
  tokensPerSecond?: number
  lastTurnDuration?: number
  lastTurnTotalTokens?: number
}

/** Cache hit ratio: cached input tokens / total prompt tokens. Null when unknown. */
export function cacheHitRatio(u: UsageStats): number | null {
  const prompt = u.promptTokens ?? 0
  const cached = u.cachedTokens ?? 0
  if (prompt <= 0 || cached <= 0) return null
  return Math.min(1, cached / prompt)
}

/** Serve (HTTPS web hosting) section of the config */
export interface ServeSection {
  enabled: boolean
  port: number
  username: string
  salt: string
  hash: string
  certPath: string
  keyPath: string
}

export interface AppConfig {
  version: string
  language: 'auto' | 'en' | 'pl' | 'fr' | 'es' | 'de' | 'it'
  theme: ThemeMode
  sandboxEnabled: boolean
  sandbox: {
    enabled: boolean
    network: 'none' | 'whitelist' | 'unrestricted'
    whitelistDomains: string[]
    timeoutSeconds: number
    memoryLimitMb: number
  }
  acpPath: string
  workingDirectory: string
  recentFolders: string[]
  lastSessionId: string | null
  agents: AgentConfig[]
  mcpServers: McpServerConfig[]
  tools: ToolConfig[]
  prompts: PromptConfig[]
  skills: SkillConfig[]
  sidebarSessionsLimit: number | null
  cachedModels: string[]
  cachedModes: Array<{ id: string; name: string }>
  dockOpen: boolean
  dockWidth: number
  collapsedProjects: string[]
  hiddenSessions: string[]
  hiddenProjects: string[]
  serve: ServeSection
  remoteConnections: RemoteConnection[]
}

export interface AgentConfig {
  id: string
  name: string
  enabled: boolean
  systemPrompt: string
  tools: string[]
  model?: string
}

export interface McpServerConfig {
  id: string
  name: string
  enabled: boolean
  command: string
  args: string[]
  env: Record<string, string>
}

export interface ToolConfig {
  id: string
  name: string
  enabled: boolean
  description: string
}

export interface PromptConfig {
  id: string
  name: string
  content: string
  variables: string[]
}

export interface SkillConfig {
  id: string
  name: string
  description: string
  command: string
}

// ===== Vibe Stats (from ~/.vibe/logs/session/*/meta.json) =====

export interface VibeSessionStats {
  steps: number
  sessionPromptTokens: number
  sessionCompletionTokens: number
  toolCallsAgreed: number
  toolCallsRejected: number
  toolCallsFailed: number
  toolCallsSucceeded: number
  sessionTotalLlmTokens: number
  sessionCachedTokens?: number
  sessionCost: number
  tokensPerSecond: number
  lastTurnDuration: number
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
  models?: string[]
}

export interface VibeToolCall {
  id: string
  index: number
  function: { name: string; arguments: string }
  type: 'function'
}

export interface VibeSessionMessage {
  role: 'user' | 'assistant' | 'tool' | 'system'
  content: string | null
  injected: boolean
  message_id: string
  tool_calls?: VibeToolCall[]
  name?: string
  tool_call_id?: string
  reasoning_content?: string | null
  reasoning_message_id?: string
}

export const defaultConfig: AppConfig = {
  version: '0.3.0',
  language: 'auto',
  theme: 'auto',
  sandboxEnabled: false,
  sandbox: {
    enabled: false,
    network: 'none',
    whitelistDomains: [],
    timeoutSeconds: 300,
    memoryLimitMb: 512
  },
  acpPath: '',
  workingDirectory: '',
  recentFolders: [],
  lastSessionId: null,
  agents: [],
  mcpServers: [],
  tools: [],
  prompts: [],
  skills: [],
  sidebarSessionsLimit: null,
  cachedModels: [],
  cachedModes: [],
  dockOpen: false,
  dockWidth: 400,
  collapsedProjects: [],
  hiddenSessions: [],
  hiddenProjects: [],
  serve: { enabled: false, port: 8765, username: '', salt: '', hash: '', certPath: '', keyPath: '' },
  remoteConnections: []
}
