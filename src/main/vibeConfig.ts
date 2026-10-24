import fs from 'fs'
import { join, basename, dirname } from 'path'
import { parse, stringify } from 'smol-toml'

// ---------------------------------------------------------------------------
// VibeConfig — mirrors the config.toml schema of the latest Mistral Vibe CLI
// (vibe/core/config/vibe_schema.py). Unknown keys are preserved via the
// index signature. Every accessor takes an optional directory so the UI can
// edit the global (~/.vibe) or the project (<cwd>/.vibe) config.
// ---------------------------------------------------------------------------

export interface VibeModelConfig {
  name: string
  provider: string
  alias?: string
  display_name?: string
  temperature?: number
  top_p?: number
  max_output_tokens?: number
  input_price?: number
  output_price?: number
  cached_input_price?: number | null
  thinking?: string
  thinking_levels?: string[]
  supports_images?: boolean
  max_context_length?: number
  auto_compact_threshold?: number
  [key: string]: unknown
}

export interface VibeProviderConfig {
  name: string
  api_key_env?: string
  api_key?: string
  base_url?: string
  [key: string]: unknown
}

export interface VibeMcpServerConfig {
  name: string
  transport?: 'http' | 'streamable-http' | 'stdio'
  url?: string
  headers?: Record<string, string>
  api_key_env?: string
  command?: string
  args?: string[]
  env?: Record<string, string>
  startup_timeout_sec?: number
  tool_timeout_sec?: number
  [key: string]: unknown
}

export interface VibeConnectorConfig {
  name: string
  disabled?: boolean
  disabled_tools?: string[]
  [key: string]: unknown
}

export interface VibeAgentFileConfig {
  name: string
  agent_type?: 'agent' | 'subagent'
  active_model?: string
  system_prompt_id?: string
  disabled_tools?: string[]
  enabled_tools?: string[]
  [key: string]: unknown
}

export interface VibeConfig {
  active_model?: string
  providers?: VibeProviderConfig[]
  models?: VibeModelConfig[]
  allowed_models?: string[]
  auto_compact_threshold?: number
  compaction_max_output_tokens?: number
  compaction_thinking?: string | null
  compaction_reasoning_limit_tokens?: number
  output_limit_retries?: number
  output_token_reserve?: number
  tools?: Record<string, Record<string, unknown>>
  tool_paths?: string[]
  enabled_tools?: string[]
  disabled_tools?: string[]
  mcp_servers?: VibeMcpServerConfig[]
  enable_connectors?: boolean
  enable_mcp?: boolean
  experimental_enable_document_library_connector?: boolean
  connectors?: VibeConnectorConfig[]
  agent_paths?: string[]
  enabled_agents?: string[]
  disabled_agents?: string[]
  installed_agents?: string[]
  default_agent?: string
  enable_auto_approve?: boolean
  skill_paths?: string[]
  enabled_skills?: string[]
  disabled_skills?: string[]
  experimental_enable_registry_skills?: boolean
  experimental_enable_model_catalog?: boolean
  enable_subagents?: boolean
  enable_background_processes?: boolean
  max_subagents?: number
  max_concurrent_subagents?: number
  bypass_tool_permissions?: boolean
  api_timeout?: number
  api_retry_max_elapsed_time?: number
  api_stream_idle_timeout?: number
  api_connect_timeout?: number
  api_write_timeout?: number
  api_pool_timeout?: number
  vibe_base_url?: string
  vibe_code_sessions_base_url?: string
  console_base_url?: string
  enable_telemetry?: boolean
  enable_otel?: boolean
  otel_endpoint?: string
  otel_redaction?: string
  theme?: string
  cursor_blink?: boolean
  show_greeting?: boolean
  show_thinking_nodes?: boolean
  show_subagent_status_list?: boolean
  disable_welcome_banner_animation?: boolean
  autocopy_to_clipboard?: boolean
  enable_mouse_capture?: boolean
  file_watcher_for_autocomplete?: boolean
  ask_confirmation_on_exit?: boolean
  voice_mode_enabled?: boolean
  narrator_enabled?: boolean
  worktree_limit?: number
  system_prompt_id?: string
  include_commit_signature?: boolean
  include_model_info?: boolean
  include_project_context?: boolean
  include_prompt_detail?: boolean
  enable_update_checks?: boolean
  enable_auto_update?: boolean
  enable_notifications?: boolean
  experimental_enable_tab_status?: boolean
  enable_system_trust_store?: boolean
  log_level?: string | null
  project_context?: { max_agents_md_files?: number; max_instructions_size?: number }
  session_logging?: { enable?: boolean; retained_sessions?: number }
  experiments?: Record<string, unknown>
  [key: string]: unknown
}

export function getVibeDirPath(): string {
  return join(process.env['HOME'] ?? process.env['USERPROFILE'] ?? '.', '.vibe')
}

/**
 * Resolve which .vibe directory a settings operation targets.
 * 'global' → ~/.vibe. 'project' → the .vibe directory of the given project
 * directory (walking up like the CLI: vibe/core/config/layers/project.py).
 * If no project config.toml exists, the project root's .vibe is returned so a
 * save creates it where the CLI would discover it.
 */
export function resolveVibeDir(scope: 'global' | 'project', cwd?: string): string {
  const globalDir = getVibeDirPath()
  if (scope !== 'project' || !cwd) return globalDir
  const home = dirname(globalDir)
  let dir: string | undefined = cwd
  for (let i = 0; i < 50 && dir; i++) {
    if (dir === home) break
    if (fs.existsSync(join(dir, '.vibe', 'config.toml'))) return join(dir, '.vibe')
    const parent = dirname(dir)
    dir = parent === dir ? undefined : parent
  }
  return join(cwd, '.vibe')
}

function configTomlPath(dir: string): string {
  return join(dir, 'config.toml')
}

function ensureDir(path: string): void {
  try {
    fs.mkdirSync(path, { recursive: true })
  } catch {}
}

// ---------------------------------------------------------------------------
// Raw config.toml access (raw TOML editor in Settings)
// ---------------------------------------------------------------------------

export function loadVibeConfigRaw(dir: string = getVibeDirPath()): { content: string; error: string | null } {
  try {
    const content = fs.readFileSync(configTomlPath(dir), 'utf-8')
    return { content, error: null }
  } catch {
    return { content: '', error: null }
  }
}

/** Validate + persist raw TOML. Returns an error string or null on success. */
export function saveVibeConfigRaw(content: string, dir: string = getVibeDirPath()): string | null {
  try {
    const parsed = parse(content)
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      return 'config.toml must be a TOML table'
    }
    ensureDir(dir)
    fs.writeFileSync(configTomlPath(dir), content, 'utf-8')
    return null
  } catch (err) {
    return err instanceof Error ? err.message : String(err)
  }
}

// ---------------------------------------------------------------------------
// Structured config access
// ---------------------------------------------------------------------------

export function loadVibeConfig(dir: string = getVibeDirPath()): VibeConfig {
  try {
    const content = fs.readFileSync(configTomlPath(dir), 'utf-8')
    return parse(content) as VibeConfig
  } catch {
    return {}
  }
}

export function saveVibeConfig(config: VibeConfig, dir: string = getVibeDirPath()): void {
  ensureDir(dir)
  fs.writeFileSync(configTomlPath(dir), stringify(config as Record<string, unknown>), 'utf-8')
}

// ---------------------------------------------------------------------------
// Agents (<dir>/agents/*.toml)
// ---------------------------------------------------------------------------

function agentsDir(dir: string): string {
  return join(dir, 'agents')
}

export function loadAgentFiles(dir: string = getVibeDirPath()): VibeAgentFileConfig[] {
  try {
    const files = fs.readdirSync(agentsDir(dir), { withFileTypes: true })
    const agents: VibeAgentFileConfig[] = []
    for (const f of files) {
      if (!f.isFile() || !f.name.endsWith('.toml')) continue
      try {
        const raw = parse(fs.readFileSync(join(agentsDir(dir), f.name), 'utf-8')) as Record<
          string,
          unknown
        >
        agents.push({
          name: (raw['name'] as string) ?? basename(f.name, '.toml'),
          agent_type: raw['agent_type'] as VibeAgentFileConfig['agent_type'],
          active_model: raw['active_model'] as string | undefined,
          system_prompt_id: raw['system_prompt_id'] as string | undefined,
          disabled_tools: raw['disabled_tools'] as string[] | undefined,
          enabled_tools: raw['enabled_tools'] as string[] | undefined,
        })
      } catch {}
    }
    return agents
  } catch {
    return []
  }
}

export function saveAgentFile(name: string, config: VibeAgentFileConfig, dir: string = getVibeDirPath()): void {
  ensureDir(agentsDir(dir))
  const filename = name.replace(/[^a-zA-Z0-9_-]/g, '_') + '.toml'
  const data: Record<string, unknown> = { ...config, name }
  fs.writeFileSync(join(agentsDir(dir), filename), stringify(data), 'utf-8')
}

export function deleteAgentFile(name: string, dir: string = getVibeDirPath()): void {
  try {
    const filename = name.replace(/[^a-zA-Z0-9_-]/g, '_') + '.toml'
    fs.unlinkSync(join(agentsDir(dir), filename))
  } catch {}
}

// ---------------------------------------------------------------------------
// Prompts (<dir>/prompts/*.md)
// ---------------------------------------------------------------------------

export interface VibePrompt {
  name: string
  content: string
}

function promptsDir(dir: string): string {
  return join(dir, 'prompts')
}

export function loadPromptFiles(dir: string = getVibeDirPath()): VibePrompt[] {
  try {
    const files = fs.readdirSync(promptsDir(dir), { withFileTypes: true })
    const prompts: VibePrompt[] = []
    for (const f of files) {
      if (!f.isFile()) continue
      if (!f.name.endsWith('.md') && !f.name.endsWith('.txt')) continue
      try {
        prompts.push({
          name: basename(f.name, f.name.endsWith('.md') ? '.md' : '.txt'),
          content: fs.readFileSync(join(promptsDir(dir), f.name), 'utf-8'),
        })
      } catch {}
    }
    return prompts
  } catch {
    return []
  }
}

export function savePromptFile(name: string, content: string, dir: string = getVibeDirPath()): void {
  ensureDir(promptsDir(dir))
  const filename = name.replace(/[^a-zA-Z0-9_-]/g, '_') + '.md'
  fs.writeFileSync(join(promptsDir(dir), filename), content, 'utf-8')
}

export function deletePromptFile(name: string, dir: string = getVibeDirPath()): void {
  try {
    const filename = name.replace(/[^a-zA-Z0-9_-]/g, '_') + '.md'
    fs.unlinkSync(join(promptsDir(dir), filename))
  } catch {}
}

// ---------------------------------------------------------------------------
// Skills (SKILL.md files, searched recursively in skill paths)
// ---------------------------------------------------------------------------

export interface VibeSkillFile {
  path: string
  full: string
  isDir: boolean
}

export interface VibeSkill {
  name: string
  content: string
  dir: string
  files: VibeSkillFile[]
}

const MAX_SKILL_DEPTH = 4

export function loadSkillFiles(dir: string = getVibeDirPath()): VibeSkill[] {
  const cfg = loadVibeConfig(dir)
  const paths = (cfg.skill_paths ?? []).map((p) =>
    String(p).replace(/^~(?=\/|$)/, process.env['HOME'] ?? process.env['USERPROFILE'] ?? '~')
  )
  paths.push(join(dir, 'skills'))

  const skills: VibeSkill[] = []
  const seen = new Set<string>()

  for (const base of paths) {
    walkSkills(base, 0, skills, seen)
  }
  return skills
}

function walkSkills(dir: string, depth: number, out: VibeSkill[], seen: Set<string>): void {
  if (depth > MAX_SKILL_DEPTH) return
  let entries: fs.Dirent[]
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    const full = join(dir, e.name)
    if (e.isDirectory()) {
      walkSkills(full, depth + 1, out, seen)
    } else if (e.isFile() && e.name.toLowerCase() === 'skill.md') {
      const name = basename(dir)
      if (seen.has(name)) continue
      try {
        out.push({ name, content: fs.readFileSync(full, 'utf-8'), dir, files: listSkillFiles(dir, '', 0) })
        seen.add(name)
      } catch {}
    }
  }
}

// Lists the files of a skill directory (depth-limited) for the settings browser
function listSkillFiles(dir: string, rel: string, depth: number): VibeSkillFile[] {
  if (depth > 2) return []
  let entries: fs.Dirent[]
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return []
  }
  entries.sort((a, b) => {
    if (a.isDirectory() !== b.isDirectory()) return a.isDirectory() ? -1 : 1
    return a.name.localeCompare(b.name)
  })
  const out: VibeSkillFile[] = []
  for (const e of entries) {
    if (e.name.startsWith('.')) continue
    const full = join(dir, e.name)
    const p = rel ? rel + '/' + e.name : e.name
    out.push({ path: p, full, isDir: e.isDirectory() })
    if (e.isDirectory()) {
      out.push(...listSkillFiles(full, p, depth + 1))
    }
  }
  return out
}
