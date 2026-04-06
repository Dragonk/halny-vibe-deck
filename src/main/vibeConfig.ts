import * as fs from 'fs'
import * as path from 'path'
import { parse, stringify } from '@iarna/toml'
import { homedir } from 'os'

export interface VibeAgentConfig {
  name: string
  agent_type?: 'agent' | 'subagent'
  active_model?: string
  system_prompt_id?: string
  disabled_tools?: string[]
  enabled_tools?: string[]
  tools?: Record<string, { permission: string }>
}

export interface VibeMcpServerConfig {
  name: string
  transport: 'http' | 'streamable-http' | 'stdio'
  url?: string
  headers?: Record<string, string>
  api_key_env?: string
  api_key_header?: string
  api_key_format?: string
  command?: string
  args?: string[]
  env?: Record<string, string>
  startup_timeout_sec?: number
  tool_timeout_sec?: number
}

export interface VibeToolConfig {
  enabled_tools?: string[]
  disabled_tools?: string[]
}

export interface VibeProviderConfig {
  name: string
  api_key_env?: string
  api_key?: string
  base_url?: string
}

export interface VibeModelConfig {
  name: string
  provider: string
  enabled?: boolean
}

export interface VibeConfig {
  agents?: VibeAgentConfig[]
  mcp_servers?: VibeMcpServerConfig[]
  tools?: VibeToolConfig
  providers?: VibeProviderConfig[]
  models?: VibeModelConfig[]
  system_prompt_id?: string
  enable_auto_update?: boolean
  enable_notifications?: boolean
}

function getVibeDir(): string {
  return path.join(homedir(), '.vibe')
}

function getVibeConfigPath(): string {
  return path.join(getVibeDir(), 'config.toml')
}

function getAgentsDir(): string {
  return path.join(getVibeDir(), 'agents')
}

function getPromptsDir(): string {
  return path.join(getVibeDir(), 'prompts')
}

function ensureVibeDir(): void {
  const vibeDir = getVibeDir()
  if (!fs.existsSync(vibeDir)) {
    fs.mkdirSync(vibeDir, { recursive: true })
  }
}

export function loadVibeConfig(): VibeConfig {
  const configPath = getVibeConfigPath()
  try {
    if (fs.existsSync(configPath)) {
      const content = fs.readFileSync(configPath, 'utf-8')
      return parse(content) as unknown as VibeConfig
    }
  } catch (e) {
    console.error('Failed to load vibe config:', e)
  }
  return {}
}

export function saveVibeConfig(config: VibeConfig): void {
  ensureVibeDir()
  const configPath = getVibeConfigPath()
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const tomlString = stringify(config as any)
  fs.writeFileSync(configPath, tomlString, 'utf-8')
}

export function loadAgentFiles(): VibeAgentConfig[] {
  const agentsDir = getAgentsDir()
  const agents: VibeAgentConfig[] = []
  try {
    if (fs.existsSync(agentsDir)) {
      const files = fs.readdirSync(agentsDir)
      for (const file of files) {
        if (file.endsWith('.toml')) {
          const content = fs.readFileSync(path.join(agentsDir, file), 'utf-8')
          const agent = parse(content) as unknown as VibeAgentConfig
          agent.name = file.replace('.toml', '')
          agents.push(agent)
        }
      }
    }
  } catch (e) {
    console.error('Failed to load agent files:', e)
  }
  return agents
}

export function saveAgentFile(name: string, config: VibeAgentConfig): void {
  ensureVibeDir()
  const agentsDir = getAgentsDir()
  if (!fs.existsSync(agentsDir)) {
    fs.mkdirSync(agentsDir, { recursive: true })
  }
  const filePath = path.join(agentsDir, `${name}.toml`)
  const { name: _, agent_type, ...configWithoutName } = config
  // Don't write agent_type if it's "agent" (only write "subagent")
  const configToSave = agent_type === 'subagent' 
    ? { ...configWithoutName, agent_type }
    : configWithoutName
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const tomlString = stringify(configToSave as any)
  fs.writeFileSync(filePath, tomlString, 'utf-8')
}

export function deleteAgentFile(name: string): void {
  const filePath = path.join(getAgentsDir(), `${name}.toml`)
  if (fs.existsSync(filePath)) {
    fs.unlinkSync(filePath)
  }
}

export function loadPromptFiles(): { name: string; content: string }[] {
  const promptsDir = getPromptsDir()
  const prompts: { name: string; content: string }[] = []
  try {
    if (fs.existsSync(promptsDir)) {
      const files = fs.readdirSync(promptsDir)
      for (const file of files) {
        if (file.endsWith('.md')) {
          const content = fs.readFileSync(path.join(promptsDir, file), 'utf-8')
          prompts.push({
            name: file.replace('.md', ''),
            content
          })
        }
      }
    }
  } catch (e) {
    console.error('Failed to load prompt files:', e)
  }
  return prompts
}

export function savePromptFile(name: string, content: string): void {
  ensureVibeDir()
  const promptsDir = getPromptsDir()
  if (!fs.existsSync(promptsDir)) {
    fs.mkdirSync(promptsDir, { recursive: true })
  }
  const filePath = path.join(promptsDir, `${name}.md`)
  fs.writeFileSync(filePath, content, 'utf-8')
}

export function deletePromptFile(name: string): void {
  const filePath = path.join(getPromptsDir(), `${name}.md`)
  if (fs.existsSync(filePath)) {
    fs.unlinkSync(filePath)
  }
}

export interface VibeSkill {
  name: string
  content: string
}

export function getVibeDirPath(): string {
  return getVibeDir()
}

export function getSkillsDir(): string {
  return path.join(getVibeDir(), 'skills')
}

export function loadSkillFiles(): VibeSkill[] {
  const skillsDir = getSkillsDir()
  const skills: VibeSkill[] = []
  try {
    if (fs.existsSync(skillsDir)) {
      const files = fs.readdirSync(skillsDir)
      for (const file of files) {
        if (file.endsWith('.md') || file.endsWith('.toml')) {
          const content = fs.readFileSync(path.join(skillsDir, file), 'utf-8')
          skills.push({
            name: file,
            content
          })
        }
      }
    }
  } catch (e) {
    console.error('Failed to load skill files:', e)
  }
  return skills
}
