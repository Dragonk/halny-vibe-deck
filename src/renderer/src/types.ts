export interface FileChange {
  path: string
  action: 'added' | 'modified' | 'deleted'
  additions?: number
  deletions?: number
}

export interface ToolCall {
  id: string
  title: string
  kind: string
  status: 'running' | 'done' | 'error'
  input?: Record<string, unknown>
  output?: string
  fileChanges?: FileChange[]
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
}

export type AppView = 'chat' | 'settings' | 'stats'

export type AcpStatus = 'disconnected' | 'connecting' | 'connected' | 'error'

export type ThemeMode = 'auto' | 'dark' | 'light'

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
  version: '0.2.0',
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
  cachedModes: []
}
