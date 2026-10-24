import { app } from 'electron'
import * as fs from 'fs'
import * as path from 'path'

export interface SandboxConfig {
  enabled: boolean
  network: 'none' | 'whitelist' | 'unrestricted'
  whitelistDomains: string[]
  timeoutSeconds: number
  memoryLimitMb: number
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

export type ThemeMode = 'auto' | 'dark' | 'light'

export interface AppConfig {
  version: string
  language: 'auto' | 'en' | 'pl' | 'fr' | 'es' | 'de' | 'it'
  theme: ThemeMode
  sandboxEnabled: boolean
  sandbox: SandboxConfig
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
  hiddenSessions: string[]
  hiddenProjects: string[]
}

const defaultConfig: AppConfig = {
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
  hiddenSessions: [],
  hiddenProjects: []
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
