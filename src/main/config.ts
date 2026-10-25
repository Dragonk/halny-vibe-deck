import { app } from 'electron'
import * as fs from 'fs'
import * as path from 'path'

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

export type ThemeMode = 'auto' | 'dark' | 'light'

/** HTTPS serve (host in browser) settings. Credentials are scrypt hashes. */
export interface ServeConfig {
  enabled: boolean
  port: number
  username: string
  /** scrypt salt (hex) */
  salt: string
  /** scrypt hash of password (hex) */
  hash: string
  /** optional custom certificate (PEM); both must be set to take effect */
  certPath: string
  keyPath: string
}

/** One SSH remote where vibe-acp runs (Codex-style remote projects). */
export interface RemoteConnection {
  id: string
  name: string
  enabled: boolean
  /** e.g. \\"ssh user@host -p 2222\\" (no remote command part) */
  sshCmd: string
  /** remote working directories shown as sub-projects */
  locations: string[]
  /** optional custom install command for vibe-acp */
  installCommand: string
}

export interface AppConfig {
  version: string
  language: 'auto' | 'en' | 'pl' | 'fr' | 'es' | 'de' | 'it'
  theme: ThemeMode
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
  autoCloseIdleMinutes?: number
  hiddenSessions: string[]
  hiddenProjects: string[]
  serve: ServeConfig
  remoteConnections: RemoteConnection[]
}

const defaultConfig: AppConfig = {
  version: '0.2.0',
  language: 'auto',
  theme: 'auto',
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
  autoCloseIdleMinutes: 60,
  hiddenSessions: [],
  hiddenProjects: [],
  serve: {
    enabled: false,
    port: 8765,
    username: '',
    salt: '',
    hash: '',
    certPath: '',
    keyPath: '',
  },
  remoteConnections: []
}

function getConfigPath(): string {
  return path.join(app.getPath('userData'), 'config.json')
}

export function loadConfig(): AppConfig {
  try {
    const raw = fs.readFileSync(getConfigPath(), 'utf-8')
    return { ...defaultConfig, ...JSON.parse(raw) }
  } catch {
    return { ...defaultConfig }
  }
}

export function saveConfig(config: AppConfig): void {
  const configPath = getConfigPath()
  fs.mkdirSync(path.dirname(configPath), { recursive: true })
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2), 'utf-8')
}
