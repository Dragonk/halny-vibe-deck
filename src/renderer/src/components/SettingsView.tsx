import React, { useState, useEffect, useCallback } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import { CodeMirrorPane } from './CodeEditor'
import DirTree from './DirTree'
import type { Language } from '../i18n'
import { isWebMode } from '../webBridge'
import { defaultConfig } from '../types'
import type { AppConfig, RemoteConnection, RemoteTestResult, ServeStatus, CliUpdateInfo } from '../types'

type Tab = 'general' | 'acp' | 'agents' | 'mcp' | 'tools' | 'prompts' | 'skills' | 'providers' | 'models' | 'raw' | 'serve' | 'remote'

// Built-in tools of the mistral-vibe CLI that work without any config.toml entry.
const DEFAULT_VIBE_TOOLS = [
  'bash',
  'edit',
  'grep',
  'read_file',
  'write_file',
  'web_fetch',
  'web_search',
  'todo',
  'task',
  'skill',
  'ask_user_question',
]

interface VibeToolPermission {
  permission: string
  allowlist?: string[]
  denylist?: string[]
  max_output_bytes?: number
  default_timeout?: number
  max_content_size?: number
  create_backup?: boolean
  fuzzy_threshold?: number
  max_read_bytes?: number
  max_write_bytes?: number
  max_content_bytes?: number
  create_parent_dirs?: boolean
  max_todos?: number
  default_max_matches?: number
  exclude_patterns?: string[]
  codeignore_file?: string
  timeout?: number
  model?: string
  max_timeout?: number
  user_agent?: string
  sample_rate?: number
  encoding?: string
  language?: string
  target_streaming_delay_ms?: number
}

interface VibeAgentConfig {
  name: string
  agent_type?: 'agent' | 'subagent'
  safety?: string
  active_model?: string
  system_prompt_id?: string
  display_name?: string
  description?: string
  instructions?: string
  disabled_tools?: string[]
  enabled_tools?: string[]
  tools?: Record<string, VibeToolPermission>
}

interface VibeMcpAuth {
  type: 'static' | 'oauth'
  headers?: Record<string, string>
  api_key_env?: string
  api_key_header?: string
  api_key_format?: string
  scopes?: string[]
  client_id?: string
  client_metadata_url?: string
  redirect_port?: number
}

interface VibeMcpServerConfig {
  name: string
  transport: 'http' | 'streamable-http' | 'stdio'
  url?: string
  headers?: Record<string, string>
  api_key_env?: string
  api_key_header?: string
  api_key_format?: string
  auth?: VibeMcpAuth
  command?: string
  args?: string[]
  env?: Record<string, string>
  cwd?: string
  startup_timeout_sec?: number
  tool_timeout_sec?: number
}

interface VibeProviderConfig {
  name: string
  api_base?: string
  api_key_env_var?: string
  api_key?: string
  api_style?: string
  backend?: string
  reasoning_field_name?: string
  project_id?: string
  region?: string
  enabled?: boolean
}

interface VibeModelConfig {
  name: string
  provider: string
  alias?: string
  display_name?: string
  supports_images?: boolean
  temperature?: number
  input_price?: number
  output_price?: number
  thinking?: 'off' | 'low' | 'medium' | 'high' | 'max'
  cached_input_price?: number | null
  top_p?: number
  max_output_tokens?: number
  thinking_levels?: string[]
  max_context_length?: number
  auto_compact_threshold?: number
  enabled?: boolean
}

interface VibeConfig {
  agents?: VibeAgentConfig[]
  mcp_servers?: VibeMcpServerConfig[]
  enabled_tools?: string[]
  disabled_tools?: string[]
  tools?: Record<string, unknown>
  providers?: VibeProviderConfig[]
  models?: VibeModelConfig[]
  system_prompt_id?: string
  enable_auto_update?: boolean
  enable_notifications?: boolean
}

interface VibePrompt {
  name: string
  content: string
}

interface VibeSkill {
  name: string
  content: string
  dir?: string
  files?: Array<{ path: string; full: string; isDir: boolean }>
}

const CLI_SCOPED_TABS = new Set(['agents', 'mcp', 'tools', 'prompts', 'skills', 'providers', 'models', 'raw'])

// Builds an auth object from legacy flat fields on old MCP configs
function mcpLegacyAuth(server: VibeMcpServerConfig): VibeMcpAuth {
  return {
    type: 'static',
    headers: server.headers || {},
    api_key_env: server.api_key_env || '',
    api_key_header: server.api_key_header || 'Authorization',
    api_key_format: server.api_key_format || 'Bearer {token}',
  }
}

// Writes auth fields and clears the legacy flat keys so the CLI does not
// reject the config (legacy keys cannot mix with an explicit auth block)
function mcpAuthUpdate(
  server: VibeMcpServerConfig,
  patch: Partial<VibeMcpAuth>,
): Partial<VibeMcpServerConfig> {
  const base = server.auth || mcpLegacyAuth(server)
  return {
    auth: staticAuth(base, patch),
    headers: undefined,
    api_key_env: undefined,
    api_key_header: undefined,
    api_key_format: undefined,
  }
}

// Strips oauth-only fields so the CLI discriminated union accepts the block
function staticAuth(auth: VibeMcpAuth, patch: Partial<VibeMcpAuth>): VibeMcpAuth {
  return {
    type: 'static',
    headers: patch.headers !== undefined ? patch.headers : auth.headers,
    api_key_env: patch.api_key_env !== undefined ? patch.api_key_env : auth.api_key_env,
    api_key_header: patch.api_key_header !== undefined ? patch.api_key_header : auth.api_key_header,
    api_key_format: patch.api_key_format !== undefined ? patch.api_key_format : auth.api_key_format,
  }
}

// Strips static-only fields for the oauth branch
function oauthAuth(auth: VibeMcpAuth, patch: Partial<VibeMcpAuth>): VibeMcpAuth {
  return {
    type: 'oauth',
    scopes: patch.scopes !== undefined ? patch.scopes : auth.scopes,
    client_id: patch.client_id !== undefined ? patch.client_id : auth.client_id,
    client_metadata_url: patch.client_metadata_url !== undefined ? patch.client_metadata_url : auth.client_metadata_url,
    redirect_port: patch.redirect_port !== undefined ? patch.redirect_port : auth.redirect_port,
  }
}

// Writes an oauth auth block and clears any legacy flat auth keys
function mcpOauthUpdate(
  server: VibeMcpServerConfig,
  patch: Partial<VibeMcpAuth>,
): Partial<VibeMcpServerConfig> {
  const base = server.auth || ({ type: 'oauth' } as VibeMcpAuth)
  return {
    auth: oauthAuth(base, patch),
    headers: undefined,
    api_key_env: undefined,
    api_key_header: undefined,
    api_key_format: undefined,
  }
}

export default function SettingsView(): React.JSX.Element {
  const { config, setConfig, setWorkingDirectory, setCurrentView, acpStatus, currentSessionWorkingDirectory, currentSessionRemoteId, openFilePreview, remoteStatuses } = useStore()
  const updateInfo = useStore((s) => s.updateInfo)
  const updateProgress = useStore((s) => s.updateProgress)
  const tr = useTranslation(config.language)
  const [tab, setTab] = useState<Tab>('general')
  const [local, setLocal] = useState({ ...config })
  const [saved, setSaved] = useState(false)
  const [sessionsLimitFocused, setSessionsLimitFocused] = useState(false)
  const [updatesBusy, setUpdatesBusy] = useState(false)
  const [updatesError, setUpdatesError] = useState<string | null>(null)
  const [remoteCli, setRemoteCli] = useState<Record<string, CliUpdateInfo | 'checking'>>({})

  const [vibeConfig, setVibeConfig] = useState<VibeConfig>({})
  const [vibeAgents, setVibeAgents] = useState<VibeAgentConfig[]>([])
  const [vibePrompts, setVibePrompts] = useState<VibePrompt[]>([])
  const [vibeSkills, setVibeSkills] = useState<VibeSkill[]>([])
  const [rawToml, setRawToml] = useState<string | null>(null)
  const [rawTomlError, setRawTomlError] = useState<string | null>(null)
  const [rawTomlSaved, setRawTomlSaved] = useState(false)
  const [configFiles, setConfigFiles] = useState<Array<{ path: string; full: string }>>([])
  const [selectedCfgFile, setSelectedCfgFile] = useState<string | null>(null)
  const [selectedCfgFileFull, setSelectedCfgFileFull] = useState<string | null>(null)
  const [rawReadyFile, setRawReadyFile] = useState<string | null>(null)
  const [vibeCfgDir, setVibeCfgDir] = useState('')
  const [loaded, setLoaded] = useState(false)
  const [selectedAgentIdx, setSelectedAgentIdx] = useState<number | null>(null)
  const [selectedMcpIdx, setSelectedMcpIdx] = useState<number | null>(null)
  const [selectedPromptIdx, setSelectedPromptIdx] = useState<number | null>(null)
  const [selectedProviderIdx, setSelectedProviderIdx] = useState<number | null>(null)
  const [selectedModelIdx, setSelectedModelIdx] = useState<number | null>(null)
  const [globalToolsCollapsed, setGlobalToolsCollapsed] = useState(false)
  const [globalModels, setGlobalModels] = useState<VibeModelConfig[]>([])
  const [selectedSkillIdx, setSelectedSkillIdx] = useState<number | null>(null)
  const [newSkillName, setNewSkillName] = useState('')
  const [newSkillFileName, setNewSkillFileName] = useState('')
  const [skillMdDraft, setSkillMdDraft] = useState('')
  const [selectedSkillFile, setSelectedSkillFile] = useState<{ path: string; full: string; isDir: boolean } | null>(null)
  const [skillFileDraft, setSkillFileDraft] = useState('')
  const [skillFileReady, setSkillFileReady] = useState<string | null>(null)
  const [skillTreeRev, setSkillTreeRev] = useState(0)
  const [skillMsg, setSkillMsg] = useState<string | null>(null)
  // ---- v0.6.0: serve (HTTPS hosting) + remote SSH connections ----
  const web = isWebMode()
  const [serveStatus, setServeStatus] = useState<ServeStatus | null>(null)
  const [serveUsername, setServeUsername] = useState('')
  const [servePassword, setServePassword] = useState('')
  const [serveCredsSaved, setServeCredsSaved] = useState(false)
  const [serveError, setServeError] = useState<string | null>(null)
  const [remoteDraft, setRemoteDraft] = useState<RemoteConnection | null>(null)
  const [remoteTestResult, setRemoteTestResult] = useState<RemoteTestResult | null>(null)
  const [remoteTesting, setRemoteTesting] = useState(false)
  const [remoteInstalling, setRemoteInstalling] = useState(false)
  const [remoteInstallMsg, setRemoteInstallMsg] = useState<string | null>(null)
  const [remoteNewLocation, setRemoteNewLocation] = useState('')
  const rawDocRef = React.useRef<(() => string) | null>(null)
  const skillDocRef = React.useRef<(() => string) | null>(null)
  const registerRawDoc = useCallback((getDoc: (() => string) | null): void => { rawDocRef.current = getDoc }, [])
  const registerSkillDoc = useCallback((getDoc: (() => string) | null): void => { skillDocRef.current = getDoc }, [])
  const [mcpJsonMode, setMcpJsonMode] = useState(false)
  const [mcpJsonText, setMcpJsonText] = useState('')
  const [mcpJsonError, setMcpJsonError] = useState<string | null>(null)
  const [scope, setScope] = useState<'global' | 'project' | 'remote'>('global')
  const projectCwd = currentSessionWorkingDirectory() || config.workingDirectory || ''
  const projectRemoteId = currentSessionRemoteId()
  const [remoteScopeId, setRemoteScopeId] = useState<string | null>(null)
  const effectiveRemoteId =
    remoteScopeId ?? (config.remoteConnections ?? []).find((c) => c.enabled)?.id ?? null
  // v0.11.2: the project scope follows the active session - when it runs on a
  // remote SSH connection, all vibe.* calls target the remote project dir.
  const vibeRemoteId = scope === 'remote'
    ? (effectiveRemoteId ?? undefined)
    : scope === 'project'
      ? (projectRemoteId ?? undefined)
      : undefined

  useEffect(() => {
    setLocal({ ...config })
  }, [config])

  const loadVibeData = useCallback(async (targetScope: 'global' | 'project' | 'remote' = scope, targetCwd: string = projectCwd): Promise<void> => {
    const targetRemoteId = targetScope === 'remote'
      ? (effectiveRemoteId ?? undefined)
      : targetScope === 'project'
        ? (currentSessionRemoteId() ?? undefined)
        : undefined
    try {
      const [cfg, agents, prompts, skills] = await Promise.all([
        window.api.vibe.getConfig(targetScope, targetCwd, targetRemoteId),
        window.api.vibe.getAgents(targetScope, targetCwd, targetRemoteId),
        window.api.vibe.getPrompts(targetScope, targetCwd, targetRemoteId),
        window.api.vibe.getSkills(targetScope, targetCwd, targetRemoteId)
      ])
      setVibeConfig(cfg as VibeConfig)
      setVibeAgents(agents as VibeAgentConfig[])
      setVibePrompts(prompts as VibePrompt[])
      setVibeSkills(skills as VibeSkill[])
      if (targetScope === 'project') {
        // Project agents may also reference models defined in the global config
        // (for a remote project that is the remote host's global config)
        const gcfg = await window.api.vibe.getConfig(
          targetRemoteId ? 'remote' : 'global',
          targetCwd,
          targetRemoteId
        )
        setGlobalModels(((gcfg as VibeConfig).models || []) as VibeModelConfig[])
      } else {
        setGlobalModels([])
      }
    } catch (err) {
      console.error('loadVibeData failed:', err)
    }
    setLoaded(true)
  }, [scope, projectCwd, effectiveRemoteId])

  useEffect(() => {
    loadVibeData()
  }, [loadVibeData])

  const handleSave = async (): Promise<void> => {
    await window.api.config.save(local)
    setConfig(local)
    setWorkingDirectory(local.workingDirectory)
    await window.api.vibe.saveConfig(vibeConfig, scope, projectCwd, vibeRemoteId)
    
    if (tab !== 'general' && acpStatus === 'connected') {
      await window.api.acp.stop()
      await new Promise(resolve => setTimeout(resolve, 1000))
      if (local.acpPath) {
        await window.api.acp.start(local.acpPath, local.workingDirectory || '')
      }
    }
    
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const handleAcpPathBrowse = async (): Promise<void> => {
    const file = await window.api.dialog.openFile()
    if (file) setLocal((prev) => ({ ...prev, acpPath: file }))
  }

  // v0.9.0: update system actions
  const handleCheckUpdates = async (): Promise<void> => {
    setUpdatesBusy(true)
    setUpdatesError(null)
    await Promise.allSettled([useStore.getState().checkAppUpdate(), useStore.getState().checkCliUpdate()])
    setUpdatesBusy(false)
  }

  const handleUpdateAppNow = async (): Promise<void> => {
    setUpdatesBusy(true)
    setUpdatesError(null)
    const res = await useStore.getState().updateAppNow()
    setUpdatesBusy(false)
    if (!res.ok && res.error) setUpdatesError(res.error)
  }

  const handleCheckRemoteCli = async (remoteId: string): Promise<void> => {
    setRemoteCli((p) => ({ ...p, [remoteId]: 'checking' }))
    try {
      const info = await useStore.getState().checkRemoteCliUpdate(remoteId)
      setRemoteCli((p) => ({ ...p, [remoteId]: info }))
    } catch (err) {
      setRemoteCli((p) => ({
        ...p,
        [remoteId]: { currentVersion: null, latestVersion: null, updateAvailable: false, error: String(err) },
      }))
    }
  }

  const handleUpdateCliNow = async (remoteId?: string): Promise<void> => {
    setUpdatesBusy(true)
    setUpdatesError(null)
    const res = await useStore.getState().updateCliNow(remoteId)
    setUpdatesBusy(false)
    if (res.ok) {
      if (remoteId) void handleCheckRemoteCli(remoteId)
    } else if (res.error) {
      setUpdatesError(res.error)
    }
  }

  const handleStartAcp = async (): Promise<void> => {
    const { setAcpStatus, setAcpError } = useStore.getState()
    if (!local.acpPath) {
      setAcpError('ACP path is not set.')
      return
    }
    setAcpStatus('connecting')
    try {
      await window.api.acp.start(local.acpPath, local.workingDirectory || '')
      setCurrentView('chat')
    } catch (err) {
      setAcpStatus('error')
      setAcpError((err as Error).message)
    }
  }

  // ---- v0.6.0: serve handlers ----
  useEffect(() => {
    if (tab === 'serve' && !web) {
      window.api.serve.status()
        .then((s) => setServeStatus(s as ServeStatus))
        .catch(console.error)
    }
  }, [tab, web])

  const handleServeToggle = async (): Promise<void> => {
    try {
      setServeError(null)
      const st = serveStatus?.running
        ? await window.api.serve.stop()
        : await window.api.serve.start()
      setServeStatus(st as ServeStatus)
    } catch (err) {
      // v0.7.0: the error must be visible in the UI (e.g. missing
      // credentials) - previously it was swallowed and the button
      // appeared to do nothing.
      console.error(err)
      setServeError((err as Error).message)
    }
  }

  const handleServeApply = async (): Promise<void> => {
    const serve = local.serve ?? defaultConfig.serve
    try {
      const st = await window.api.serve.saveConfig({
        enabled: serve.enabled,
        port: serve.port,
        certPath: serve.certPath || undefined,
        keyPath: serve.keyPath || undefined,
      })
      setServeStatus(st as ServeStatus)
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (err) {
      console.error(err)
    }
  }

  const handleServeCredentials = async (): Promise<void> => {
    if (!serveUsername || !servePassword) return
    try {
      await window.api.serve.setCredentials(serveUsername, servePassword)
      setServeCredsSaved(true)
      setServePassword('')
      setTimeout(() => setServeCredsSaved(false), 2500)
    } catch (err) {
      console.error(err)
    }
  }

  const handleServeCertBrowse = async (which: 'certPath' | 'keyPath'): Promise<void> => {
    const file = await window.api.dialog.openFile()
    if (file) {
      setLocal((p) => ({ ...p, serve: { ...(p.serve ?? defaultConfig.serve), [which]: file } }))
    }
  }

  // ---- v0.6.0: remote SSH handlers ----
  const handleRemoteTest = async (): Promise<void> => {
    if (!remoteDraft) return
    setRemoteTesting(true)
    setRemoteTestResult(null)
    setRemoteInstallMsg(null)
    try {
      const res = (await window.api.remote.test(remoteDraft)) as RemoteTestResult
      setRemoteTestResult(res)
    } catch (err) {
      setRemoteTestResult({
        reachable: false,
        acpInstalled: false,
        acpPath: null,
        version: null,
        error: (err as Error).message,
      })
    } finally {
      setRemoteTesting(false)
    }
  }

  const handleRemoteInstall = async (): Promise<void> => {
    if (!remoteDraft) return
    setRemoteInstalling(true)
    setRemoteInstallMsg(null)
    try {
      // Install needs a saved connection id — persist first, then install.
      const saved = (await window.api.remote.save(remoteDraft)) as RemoteConnection
      setRemoteDraft(saved)
      await window.api.remote.installAcp(saved.id)
      setRemoteInstallMsg(tr.remoteInstallOk)
      void handleRemoteTest()
    } catch (err) {
      setRemoteInstallMsg((err as Error).message)
    } finally {
      setRemoteInstalling(false)
    }
  }

  const handleRemoteSave = async (): Promise<void> => {
    if (!remoteDraft) return
    try {
      await window.api.remote.save(remoteDraft)
      const cfg = (await window.api.config.get()) as AppConfig
      setConfig(cfg)
      setLocal((p) => ({ ...p, remoteConnections: cfg.remoteConnections }))
      setRemoteDraft(null)
      void useStore.getState().refreshRemoteData()
    } catch (err) {
      console.error(err)
    }
  }

  const handleRemoteDelete = async (id: string): Promise<void> => {
    try {
      await window.api.remote.delete(id)
      const cfg = (await window.api.config.get()) as AppConfig
      setConfig(cfg)
      setLocal((p) => ({ ...p, remoteConnections: cfg.remoteConnections }))
      void useStore.getState().refreshRemoteData()
    } catch (err) {
      console.error(err)
    }
  }

  // ---- Skills: create / delete / import / edit files ----
  useEffect(() => {
    const skill = selectedSkillIdx !== null ? vibeSkills[selectedSkillIdx] : undefined
    setSkillMdDraft(skill?.content ?? '')
    // Keep the edited file selected across reloads; default to SKILL.md.
    const stillThere =
      selectedSkillFile && skill?.files?.some((f) => f.full === selectedSkillFile.full)
    if (skill && !stillThere) {
      const md = skill.files?.find((f) => !f.isDir && f.path === 'SKILL.md')
      setSelectedSkillFile(md ? { path: 'SKILL.md', full: md.full, isDir: false } : null)
      setSkillFileDraft('')
    } else if (!skill) {
      setSelectedSkillFile(null)
      setSkillFileDraft('')
    }
    setSkillTreeRev((r) => r + 1)
  }, [selectedSkillIdx, vibeSkills])

  useEffect(() => {
    if (!selectedSkillFile || selectedSkillFile.isDir) return
    let cancelled = false
    setSkillFileReady(null)
    window.api.fs.readText(selectedSkillFile.full, vibeRemoteId).then((res) => {
      if (cancelled) return
      setSkillFileDraft(res.error ? '' : res.content)
      setSkillFileReady(selectedSkillFile.full)
    }).catch(console.error)
    return () => { cancelled = true }
  }, [selectedSkillFile])

  const flashSkillMsg = (msg: string): void => {
    setSkillMsg(msg)
    window.setTimeout(() => setSkillMsg(null), 2500)
  }

  const handleCreateSkill = async (): Promise<void> => {
    const name = newSkillName.trim()
    if (!name) return
    const err = await window.api.vibe.createSkill(name, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    setNewSkillName('')
    await loadVibeData()
    const fresh = (await window.api.vibe.getSkills(scope, projectCwd)) as VibeSkill[]
    const idx = fresh.findIndex((s) => s.name.toLowerCase() === name.toLowerCase())
    setSelectedSkillIdx(idx >= 0 ? idx : null)
  }

  const handleDeleteSkill = async (skill: VibeSkill): Promise<void> => {
    if (!window.confirm(tr.deleteSkillConfirm.replace('{{name}}', skill.name))) return
    const err = await window.api.vibe.deleteSkill(skill.name, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    setSelectedSkillIdx(null)
    await loadVibeData()
  }

  const handleSaveSkillMd = async (skill: VibeSkill): Promise<void> => {
    const err = await window.api.vibe.writeSkillFile(skill.name, 'SKILL.md', skillMdDraft, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    await loadVibeData()
  }

  const handleAddSkillFile = async (skill: VibeSkill): Promise<void> => {
    const rel = newSkillFileName.trim()
    if (!rel) return
    const err = await window.api.vibe.writeSkillFile(skill.name, rel, '', scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    setNewSkillFileName('')
    await loadVibeData()
  }

  const handleSaveSkillFile = async (skill: VibeSkill): Promise<void> => {
    if (!selectedSkillFile) return
    const content = skillDocRef.current?.() ?? skillFileDraft
    const err = await window.api.vibe.writeSkillFile(skill.name, selectedSkillFile.path, content, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    await loadVibeData()
  }

  const handleDeleteSkillFileBtn = async (skill: VibeSkill, relPath: string): Promise<void> => {
    if (!window.confirm(tr.deleteFileConfirm.replace('{{path}}', relPath))) return
    const err = await window.api.vibe.deleteSkillFile(skill.name, relPath, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    if (selectedSkillFile?.path === relPath) setSelectedSkillFile(null)
    await loadVibeData()
  }

  const handleImportSkillFolder = async (): Promise<void> => {
    const folder = await window.api.dialog.openFolder()
    if (!folder) return
    const err = await window.api.vibe.importSkill(folder, false, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    await loadVibeData()
  }

  const handleImportSkillZip = async (): Promise<void> => {
    const file = await window.api.dialog.openFile()
    if (!file) return
    const err = await window.api.vibe.importSkill(file, true, scope, projectCwd, vibeRemoteId)
    if (err) { flashSkillMsg(tr.skillImportFailed.replace('{{error}}', err)); return }
    await loadVibeData()
  }

  const handleOpenSkillsFolder = async (): Promise<void> => {
    const dir = await window.api.vibe.getDir(scope, projectCwd, vibeRemoteId)
    if (scope === 'remote' || vibeRemoteId) return
    const skillsDir = `${dir}/skills`
    await window.api.shell.openPath(skillsDir)
  }

  const loadRawToml = useCallback(async (): Promise<void> => {
    setRawTomlError(null)
    setRawReadyFile(null)
    const res = await window.api.vibe.getRawConfig(scope, projectCwd, vibeRemoteId)
    if (res.error) setRawTomlError(res.error)
    else setRawTomlError(null)
    setRawToml(res.content)
    setRawReadyFile('__config__')
  }, [scope, projectCwd, vibeRemoteId])

  // Reload the raw TOML whenever the scope (or project cwd) changes —
  // the textarea must always show the config of the current scope.
  useEffect(() => {
    if (tab !== 'raw') return
    setSelectedCfgFile(null)
    setSelectedCfgFileFull(null)
    setRawToml(null)
    setRawReadyFile(null)
    void loadRawToml()
  }, [scope, projectCwd, vibeRemoteId])

  // Show the .vibe directory tree of the current scope next to the editor.
  useEffect(() => {
    if (tab !== 'raw') return
    window.api.vibe
      .getDir(scope, projectCwd, vibeRemoteId)
      .then((dir) => setVibeCfgDir(dir))
      .catch(console.error)
  }, [tab, scope, projectCwd, vibeRemoteId])

  const loadConfigFile = useCallback(async (rel: string | null, full?: string | null): Promise<void> => {
    setSelectedCfgFile(rel)
    setRawTomlError(null)
    setRawReadyFile(null)
    if (rel === null) {
      setSelectedCfgFileFull(null)
      await loadRawToml()
      return
    }
    const target = full ?? configFiles.find((x) => x.path === rel)?.full ?? null
    if (!target) return
    setSelectedCfgFileFull(target)
    const res = await window.api.fs.readText(target, vibeRemoteId)
    if (res.error) {
      setRawTomlError(res.error)
      return
    }
    setRawToml(res.content)
    setRawReadyFile(target)
  }, [configFiles, loadRawToml])

  const handleSaveRawToml = async (): Promise<void> => {
    const content = rawDocRef.current?.() ?? rawToml
    if (content === null) return
    let err: string | null = null
    if (selectedCfgFile !== null) {
      const target = selectedCfgFileFull ?? configFiles.find((x) => x.path === selectedCfgFile)?.full ?? null
      if (target) err = (await window.api.fs.writeText(target, content, vibeRemoteId)).error
    } else {
      err = await window.api.vibe.saveRawConfig(content, scope, projectCwd, vibeRemoteId)
    }
    if (err) {
      setRawTomlError(err)
      return
    }
    setRawTomlError(null)
    setRawTomlSaved(true)
    setTimeout(() => setRawTomlSaved(false), 2000)
    await loadVibeData()
  }

  const handleRenameAgent = async (oldName: string, newName: string, agentConfig: VibeAgentConfig): Promise<void> => {
    if (newName && newName !== oldName) {
      await window.api.vibe.deleteAgent(oldName, scope, projectCwd, vibeRemoteId)
      await window.api.vibe.saveAgent(newName, agentConfig, scope, projectCwd, vibeRemoteId)
      await loadVibeData()
    }
  }

  const handleSaveAgent = async (name: string, agentConfig: VibeAgentConfig): Promise<void> => {
    await window.api.vibe.saveAgent(name, agentConfig, scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleDeleteAgent = async (name: string): Promise<void> => {
    await window.api.vibe.deleteAgent(name, scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleAddAgent = async (): Promise<void> => {
    const name = `agent-${Date.now()}`
    const newAgent: VibeAgentConfig = {
      name,
      agent_type: 'agent'
    }
    await window.api.vibe.saveAgent(name, newAgent, scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleAddToolPermission = (agentName: string | null, toolName: string): void => {
    if (agentName) {
      const agent = vibeAgents.find(a => a.name === agentName)
      if (agent) {
        const tools = agent.tools || {}
        tools[`tools.${toolName}`] = { permission: 'ask' }
        handleSaveAgent(agentName, { ...agent, tools })
      }
    } else {
      const tools = vibeConfig.tools || {}
      tools[`tools.${toolName}`] = { permission: 'ask' }
      setVibeConfig({ ...vibeConfig, tools })
    }
  }

  const handleUpdateToolPermission = (agentName: string | null, toolName: string, updates: Partial<VibeToolPermission>): void => {
    if (agentName) {
      const agent = vibeAgents.find(a => a.name === agentName)
      if (agent) {
        const tools = agent.tools || {}
        const existing = tools[toolName] || { permission: 'ask' }
        tools[toolName] = { ...existing, ...updates }
        handleSaveAgent(agentName, { ...agent, tools })
      }
    } else {
      const tools = vibeConfig.tools || {}
      const existing = tools[toolName] || { permission: 'ask' }
      tools[toolName] = { ...existing, ...updates }
      setVibeConfig({ ...vibeConfig, tools })
    }
  }

  const handleRemoveToolPermission = (agentName: string | null, toolName: string): void => {
    if (agentName) {
      const agent = vibeAgents.find(a => a.name === agentName)
      if (agent) {
        const tools = { ...agent.tools }
        delete tools[toolName]
        handleSaveAgent(agentName, { ...agent, tools })
      }
    } else {
      const tools = { ...vibeConfig.tools }
      delete tools[toolName]
      setVibeConfig({ ...vibeConfig, tools })
    }
  }

  const handleSavePrompt = async (oldName: string, newName: string, content: string): Promise<void> => {
    if (newName !== oldName) {
      await window.api.vibe.deletePrompt(oldName, scope, projectCwd, vibeRemoteId)
    }
    await window.api.vibe.savePrompt(newName, content, scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleDeletePrompt = async (name: string): Promise<void> => {
    await window.api.vibe.deletePrompt(name, scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleAddPrompt = async (): Promise<void> => {
    const name = `prompt-${Date.now()}`
    await window.api.vibe.savePrompt(name, '# New Prompt\n\nWrite your prompt here with {{variables}}.', scope, projectCwd, vibeRemoteId)
    await loadVibeData()
  }

  const handleAddMcpServer = (): void => {
    const newServers = [...(vibeConfig.mcp_servers || [])]
    newServers.push({
      name: `server-${Date.now()}`,
      transport: 'stdio' as const,
      command: '',
      args: []
    })
    setVibeConfig({ ...vibeConfig, mcp_servers: newServers })
  }

  const handleUpdateMcpServer = (idx: number, updates: Partial<VibeMcpServerConfig>): void => {
    const newServers = [...(vibeConfig.mcp_servers || [])]
    newServers[idx] = { ...newServers[idx], ...updates }
    setVibeConfig({ ...vibeConfig, mcp_servers: newServers })
  }

  const handleRemoveMcpServer = (idx: number): void => {
    const newServers = (vibeConfig.mcp_servers || []).filter((_, i) => i !== idx)
    setVibeConfig({ ...vibeConfig, mcp_servers: newServers })
  }

  const handleAddProvider = (): void => {
    const newProviders = [...(vibeConfig.providers || [])]
    newProviders.push({
      name: `provider-${Date.now()}`,
      api_base: '',
      api_key_env_var: '',
      enabled: true
    })
    setVibeConfig({ ...vibeConfig, providers: newProviders })
  }

  const handleUpdateProvider = (idx: number, updates: Partial<VibeProviderConfig>): void => {
    const newProviders = [...(vibeConfig.providers || [])]
    newProviders[idx] = { ...newProviders[idx], ...updates }
    setVibeConfig({ ...vibeConfig, providers: newProviders })
  }

  const handleRemoveProvider = (idx: number): void => {
    const newProviders = (vibeConfig.providers || []).filter((_, i) => i !== idx)
    setVibeConfig({ ...vibeConfig, providers: newProviders })
  }

  const handleAddModel = (): void => {
    const newModels = [...(vibeConfig.models || [])]
    newModels.push({
      name: `model-${Date.now()}`,
      provider: vibeConfig.providers?.[0]?.name || 'mistral',
      alias: '',
      temperature: 0.2,
      thinking: 'off',
      enabled: true
    })
    setVibeConfig({ ...vibeConfig, models: newModels })
  }

  const handleUpdateModel = (idx: number, updates: Partial<VibeModelConfig>): void => {
    const newModels = [...(vibeConfig.models || [])]
    newModels[idx] = { ...newModels[idx], ...updates }
    setVibeConfig({ ...vibeConfig, models: newModels })
  }

  const handleRemoveModel = (idx: number): void => {
    const newModels = (vibeConfig.models || []).filter((_, i) => i !== idx)
    setVibeConfig({ ...vibeConfig, models: newModels })
  }

  const getToolPermissions = (agentName: string | null): Record<string, VibeToolPermission> => {
    if (agentName) {
      const agent = vibeAgents.find(a => a.name === agentName)
      return (agent?.tools || {}) as Record<string, VibeToolPermission>
    }
    return (vibeConfig.tools as Record<string, VibeToolPermission>) || {}
  }

  const renderToolPermissionRow = (agentName: string | null, toolName: string, toolPerm: VibeToolPermission) => {
    const cleanName = toolName.replace('tools.', '')
    return (
      <div key={toolName} className="tool-perm-config-row">
        <input
          className="field-input"
          type="text"
          defaultValue={cleanName}
          onBlur={(e) => {
            const oldKey = toolName
            const newKey = `tools.${e.target.value}`
            if (oldKey !== newKey) {
              handleRemoveToolPermission(agentName, oldKey)
              handleUpdateToolPermission(agentName, newKey, toolPerm)
            }
          }}
          placeholder="tool_name"
        />
        <select
          className="field-select"
          value={toolPerm.permission || 'ask'}
          onChange={(e) => handleUpdateToolPermission(agentName, toolName, { permission: e.target.value })}
        >
          <option value="always">{tr.always || 'Always'}</option>
          <option value="ask">{tr.ask || 'Ask'}</option>
          <option value="never">{tr.never || 'Never'}</option>
        </select>
        <button
          className="field-btn danger"
          onClick={() => handleRemoveToolPermission(agentName, toolName)}
        >
          {tr.remove}
        </button>
      </div>
    )
  }

  const renderToolConfigSection = (agentName: string | null) => {
    const toolPerms = getToolPermissions(agentName)
    const configuredTools = Object.keys(toolPerms)
    const availableTools = vibeConfig.tools ? Object.keys(vibeConfig.tools) : []

    return (
      <div className="tool-permissions-section">
        <div className="field">
          <label className="field-label">{tr.toolPermissions || 'Tool Permissions'}</label>
          {configuredTools.length > 0 ? (
            <div className="tool-perm-list">
              {configuredTools.map(toolName => 
                renderToolPermissionRow(agentName, toolName, toolPerms[toolName])
              )}
            </div>
          ) : (
            <div>
              <p className="empty-text">{tr.noToolsConfigured}</p>
              <p className="default-tools-note">{tr.defaultToolsHint}</p>
              <div className="tool-perm-list">
                {DEFAULT_VIBE_TOOLS.filter(
                  (t) =>
                    !configuredTools.includes(t) &&
                    !configuredTools.includes('tools.' + t) &&
                    !toolPerms[t]
                ).map((t) => renderToolPermissionRow(agentName, t, { permission: 'ask' }))}
              </div>
            </div>
          )}
        </div>
        <div className="field">
          <label className="field-label">{tr.addTool || 'Add Tool'}</label>
          <div className="add-tool-row">
            <input
              className="field-input"
              type="text"
              id={`new-tool-${agentName || 'global'}`}
              placeholder={tr.toolNamePlaceholder || 'tool_name'}
              style={{ flex: 1 }}
            />
            <select
              className="field-select"
              id={`new-tool-perm-${agentName || 'global'}`}
              defaultValue="ask"
              style={{ width: 'auto' }}
            >
              <option value="always">{tr.always || 'Always'}</option>
              <option value="ask">{tr.ask || 'Ask'}</option>
              <option value="never">{tr.never || 'Never'}</option>
            </select>
            <button
              className="field-btn"
              onClick={() => {
                const input = document.getElementById(`new-tool-${agentName || 'global'}`) as HTMLInputElement
                const select = document.getElementById(`new-tool-perm-${agentName || 'global'}`) as HTMLSelectElement
                if (input?.value) {
                  handleUpdateToolPermission(agentName, `tools.${input.value}`, { permission: select.value })
                  input.value = ''
                }
              }}
            >
              {tr.add || 'Add'}
            </button>
          </div>
        </div>
        {availableTools.length > 0 && (
          <div className="field">
            <label className="field-label">{tr.copyFromGlobal || 'Copy from global'}</label>
            <div className="copy-tools-row">
              {availableTools.map(toolName => (
                <button
                  key={toolName}
                  className="field-btn"
                  onClick={() => {
                    const globalPerm = vibeConfig.tools?.[toolName]
                    if (globalPerm) {
                      handleUpdateToolPermission(agentName, toolName, { ...globalPerm })
                    }
                  }}
                >
                  {toolName.replace('tools.', '')}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="settings-view">
      <div className="settings-header">
        <div className="settings-title-row">
          <h2 className="settings-title">{tr.settings}</h2>
          {CLI_SCOPED_TABS.has(tab) && (
            <div className="scope-switch">
              <button
                className={`scope-pill ${scope === 'global' ? 'active' : ''}`}
                onClick={() => { setScope('global'); setRawToml(null) }}
              >
                {tr.settingsScopeGlobal}
              </button>
              {(config.remoteConnections ?? []).filter((c) => c.enabled).map((c) => (
                <button
                  key={'scp-' + c.id}
                  className={`scope-pill ${scope === 'remote' && effectiveRemoteId === c.id ? 'active' : ''}`}
                  onClick={() => { setScope('remote'); setRemoteScopeId(c.id); setRawToml(null) }}
                >
                  📍 {c.name}
                </button>
              ))}
              <button
                className={`scope-pill ${scope === 'project' ? 'active' : ''}`}
                disabled={!projectCwd}
                title={projectCwd ? (projectRemoteId ? '📍 ' + projectCwd + '/.vibe' : projectCwd + '/.vibe') : ''}
                onClick={() => { setScope('project'); setRawToml(null) }}
              >
                {projectRemoteId ? '📍 ' : ''}{tr.settingsScopeProject}
              </button>
            </div>
          )}
          {CLI_SCOPED_TABS.has(tab) && (
            <span className="scope-path">
              {scope === 'project'
                ? (projectRemoteId
                    ? '📍 ' + ((config.remoteConnections ?? []).find((c) => c.id === projectRemoteId)?.name ?? 'remote') + ': ' + projectCwd + '/.vibe'
                    : projectCwd + '/.vibe')
                : scope === 'remote'
                  ? ((config.remoteConnections ?? []).find((c) => c.id === effectiveRemoteId)?.name ?? 'remote') + ': ~/.vibe'
                  : '~/.vibe'}
            </span>
          )}
        </div>
        <div className="settings-tabs">
          <button
            className={`settings-tab ${tab === 'general' ? 'active' : ''}`}
            onClick={() => setTab('general')}
          >
            {tr.general}
          </button>
          {!web && (
            <button
              className={`settings-tab ${tab === 'serve' ? 'active' : ''}`}
              onClick={() => setTab('serve')}
            >
              {tr.serveTab}
            </button>
          )}
          <button
            className={`settings-tab ${tab === 'remote' ? 'active' : ''}`}
            onClick={() => setTab('remote')}
          >
            {tr.remoteTab}
          </button>
          <button
            className={`settings-tab ${tab === 'agents' ? 'active' : ''}`}
            onClick={() => setTab('agents')}
          >
            {tr.agents}
          </button>
          <button
            className={`settings-tab ${tab === 'mcp' ? 'active' : ''}`}
            onClick={() => setTab('mcp')}
          >
            {tr.mcp}
          </button>
          <button
            className={`settings-tab ${tab === 'tools' ? 'active' : ''}`}
            onClick={() => setTab('tools')}
          >
            {tr.tools}
          </button>
          <button
            className={`settings-tab ${tab === 'prompts' ? 'active' : ''}`}
            onClick={() => setTab('prompts')}
          >
            {tr.prompts}
          </button>
          <button
            className={`settings-tab ${tab === 'skills' ? 'active' : ''}`}
            onClick={() => setTab('skills')}
          >
            {tr.skills}
          </button>
          <button
            className={`settings-tab ${tab === 'providers' ? 'active' : ''}`}
            onClick={() => setTab('providers')}
          >
            {tr.providers}
          </button>
          <button
            className={`settings-tab ${tab === 'models' ? 'active' : ''}`}
            onClick={() => setTab('models')}
          >
            {tr.models}
          </button>
          <button
            className={`settings-tab ${tab === 'raw' ? 'active' : ''}`}
            onClick={() => {
              setTab('raw')
              if (rawToml === null) loadRawToml()
            }}
          >
            {tr.rawConfig}
          </button>
        </div>
      </div>

      <div className="settings-content">
        {tab === 'general' && (
          <div className="settings-section">
            <div className="field">
              <label className="field-label">{tr.language}</label>
              <select
                className="field-select"
                value={local.language}
                onChange={(e) => setLocal((p) => ({ ...p, language: e.target.value as Language }))}
              >
                <option value="auto">{tr.autoSystem}</option>
                <option value="en">{tr.english}</option>
                <option value="pl">{tr.polish}</option>
                <option value="fr">{tr.french}</option>
                <option value="es">{tr.spanish}</option>
                <option value="de">{tr.german}</option>
                <option value="it">{tr.italian}</option>
              </select>
            </div>

            <div className="field">
              <label className="field-label">{tr.theme}</label>
              <select
                className="field-select"
                value={local.theme}
                onChange={(e) => setLocal((p) => ({ ...p, theme: e.target.value as 'auto' | 'dark' | 'light' }))}
              >
                <option value="auto">{tr.autoSystem}</option>
                <option value="dark">{tr.dark}</option>
                <option value="light">{tr.light}</option>
              </select>
            </div>

            <div className="field">
              <label className="field-label">{tr.sidebarSessionsLimit}</label>
              <div className="field-row">
                <button
                  className="field-btn"
                  onClick={() => setLocal((p) => ({ ...p, sidebarSessionsLimit: Math.max(0, (p.sidebarSessionsLimit ?? 0) - 1) }))}
                >
                  −
                </button>
                <input
                  className="field-input"
                  type="text"
                  onFocus={(e) => {
                    setSessionsLimitFocused(true)
                    e.target.value = String(local.sidebarSessionsLimit ?? 0)
                    e.target.select()
                  }}
                  onBlur={(e) => {
                    setSessionsLimitFocused(false)
                    const num = parseInt(e.target.value, 10)
                    setLocal((p) => ({ ...p, sidebarSessionsLimit: isNaN(num) || num < 0 ? 0 : num }))
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.currentTarget.blur()
                    }
                  }}
                  value={sessionsLimitFocused ? undefined : (local.sidebarSessionsLimit === 0 || local.sidebarSessionsLimit === null ? tr.allSessions : local.sidebarSessionsLimit)}
                  placeholder={tr.allSessions}
                  style={{ width: 80, textAlign: 'center' }}
                />
                <button
                  className="field-btn"
                  onClick={() => setLocal((p) => ({ ...p, sidebarSessionsLimit: ((p.sidebarSessionsLimit ?? 0) + 1) || 1 }))}
                >
                  +
                </button>
              </div>
            </div>
          </div>
        )}

        {tab === 'general' && (
          <div className="settings-section">
            <div className="field">
              <label className="field-label">{tr.vibeAcpPath}</label>
              <p className="field-hint" dangerouslySetInnerHTML={{ __html: tr.vibeAcpHint.replace('uv tool install mistral-vibe', '<code>uv tool install mistral-vibe</code>') }} />
              <div className="field-row">
                <input
                  className="field-input"
                  type="text"
                  value={local.acpPath}
                  onChange={(e) => setLocal((p) => ({ ...p, acpPath: e.target.value }))}
                  placeholder={tr.vibeAcpPlaceholder}
                />
                <button className="field-btn" onClick={handleAcpPathBrowse}>
                  {tr.browse}
                </button>
              </div>
            </div>

            <div className="field">
              <label className="field-label">{tr.autoCloseIdleLabel}</label>
              <input
                className="field-input"
                type="number"
                min={0}
                max={10080}
                value={local.autoCloseIdleMinutes ?? 60}
                onChange={(e) =>
                  setLocal((p) => ({
                    ...p,
                    autoCloseIdleMinutes: Math.max(0, parseInt(e.target.value, 10) || 0),
                  }))
                }
              />
            </div>

            <div className="field">
              <button className="action-btn" onClick={handleStartAcp}>
                {tr.connectToAgent}
              </button>
              <button
                className="action-btn danger"
                onClick={() => window.api.acp.stop().catch(console.error)}
                style={{ marginLeft: 8 }}
              >
                {tr.disconnect}
              </button>
            </div>
          </div>
        )}

        {tab === 'general' && (
          <div className="settings-section">
            <h3 className="settings-section-title">{tr.updatesSectionTitle}</h3>
            <div className="updates-row">
              <div className="updates-info">
                <span className="updates-name">{tr.updatesAppLabel}</span>
                <span className="updates-versions">
                  {tr.updatesCurrentLabel.replace('{{version}}', updateInfo.app?.currentVersion ?? '?')}
                  {updateInfo.app?.latestVersion
                    ? ' · ' + tr.updatesLatestLabel.replace('{{version}}', updateInfo.app.latestVersion)
                    : ''}
                </span>
                {updateProgress && (
                  <span className="updates-progress">
                    {updateProgress.phase === 'download'
                      ? tr.updateProgressDownload
                          .replace('{{percent}}', String(updateProgress.percent ?? '…'))
                          .replace('{{received}}', (updateProgress.received / 1048576).toFixed(1))
                          .replace(
                            '{{total}}',
                            updateProgress.total != null ? (updateProgress.total / 1048576).toFixed(1) : '?',
                          )
                      : updateProgress.phase === 'install'
                        ? tr.updateProgressInstall
                        : tr.updateProgressRestart}
                  </span>
                )}
              </div>
              <div className="updates-actions">
                <button className="field-btn" disabled={updatesBusy} onClick={() => void handleCheckUpdates()}>
                  {tr.updatesCheckButton}
                </button>
                <button
                  className="action-btn"
                  disabled={web || updatesBusy || !(updateInfo.app?.updateAvailable ?? false)}
                  onClick={() => void handleUpdateAppNow()}
                >
                  {tr.updatesUpdateButton}
                </button>
              </div>
            </div>
            <p className="field-hint">{tr.updatesAppRestartHint}</p>
            <div className="updates-row">
              <div className="updates-info">
                <span className="updates-name">{tr.updatesCliLabel}</span>
                <span className="updates-versions">
                  {tr.updatesCurrentLabel.replace('{{version}}', updateInfo.cli?.currentVersion ?? '?')}
                  {updateInfo.cli?.latestVersion
                    ? ' · ' + tr.updatesLatestLabel.replace('{{version}}', updateInfo.cli.latestVersion)
                    : ''}
                </span>
              </div>
              <div className="updates-actions">
                <button
                  className="action-btn"
                  disabled={updatesBusy || !(updateInfo.cli?.updateAvailable ?? false)}
                  onClick={() => void handleUpdateCliNow()}
                >
                  {tr.updatesUpdateButton}
                </button>
              </div>
            </div>
            <p className="field-hint">{tr.updatesUpdateHint}</p>
            <div className="remote-cli-updates">
              <label className="field-label">{tr.updatesRemoteCliTitle}</label>
              {(local.remoteConnections ?? []).filter((c) => c.enabled).length === 0 && (
                <p className="field-hint">{tr.remoteNoConnections}</p>
              )}
              {(local.remoteConnections ?? []).filter((c) => c.enabled).map((conn) => {
                const info = remoteCli[conn.id]
                return (
                  <div key={conn.id} className="updates-row">
                    <div className="updates-info">
                      <span className="updates-name">{conn.name}</span>
                      <span className="updates-versions">
                        {info === 'checking'
                          ? tr.updatesChecking
                          : info
                            ? tr.updatesCurrentLabel.replace('{{version}}', info.currentVersion ?? '?') +
                              ' · ' +
                              tr.updatesLatestLabel.replace('{{version}}', info.latestVersion ?? '?')
                            : tr.updatesNotChecked}
                      </span>
                    </div>
                    <div className="updates-actions">
                      <button
                        className="field-btn"
                        disabled={updatesBusy}
                        onClick={() => void handleCheckRemoteCli(conn.id)}
                      >
                        {tr.updatesCheckButton}
                      </button>
                      <button
                        className="action-btn"
                        disabled={
                          updatesBusy || !info || info === 'checking' || !info.updateAvailable
                        }
                        onClick={() => void handleUpdateCliNow(conn.id)}
                      >
                        {tr.updatesUpdateButton}
                      </button>
                    </div>
                  </div>
                )
              })}
            </div>
            {updatesError && <p className="field-hint">{updatesError}</p>}
          </div>
        )}

        {tab === 'agents' && (
          <div className="settings-section tools-section">
            <div className="tools-config-panel">
              <div className="settings-section-header" style={{ width: '100%', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <h3 className="settings-section-title">{tr.agents}</h3>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <button className="field-btn" onClick={handleAddAgent}>
                    {tr.addAgent}
                  </button>
                  <button
                    className="toggle-tools-btn"
                    onClick={() => setGlobalToolsCollapsed(!globalToolsCollapsed)}
                    title={globalToolsCollapsed ? tr.showTools : tr.hideTools}
                  >
                    {globalToolsCollapsed ? '◀' : '▶'}
                  </button>
                </div>
              </div>
              {!loaded ? (
                <p className="empty-text">{tr.loading}</p>
              ) : vibeAgents.length === 0 ? (
                <p className="empty-text">{tr.noAgentsConfigured}</p>
              ) : (
                <div className="master-detail">
                  <div className="master-list">
                    {vibeAgents.map((agent, idx) => (
                      <div
                        key={agent.name}
                        className={`master-item ${selectedAgentIdx === idx ? 'selected' : ''}`}
                        onClick={() => setSelectedAgentIdx(idx)}
                      >
                        <span className="master-item-name">{agent.name}</span>
                        <span className="master-item-type">{agent.agent_type || 'agent'}</span>
                      </div>
                    ))}
                  </div>
                  <div className="master-detail-panel">
                    {selectedAgentIdx !== null && vibeAgents[selectedAgentIdx] ? (() => {
                      const agent = vibeAgents[selectedAgentIdx]
                      const isSubagent = (agent.agent_type || 'agent') === 'subagent'
                      return (
                        <div>
                          <div className="field">
                            <label className="field-label">{tr.name}</label>
                            <div className="config-item-header-row">
                              <input
                                key={agent.name}
                                className="field-input"
                                type="text"
                                defaultValue={agent.name}
                                onBlur={(e) => {
                                  if (e.target.value !== agent.name) {
                                    handleRenameAgent(agent.name, e.target.value, agent)
                                  }
                                }}
                                placeholder={tr.agentName}
                              />
                              <select
                                className="field-select"
                                value={agent.agent_type || 'agent'}
                                onChange={(e) => {
                                  handleSaveAgent(agent.name, { ...agent, agent_type: e.target.value as 'agent' | 'subagent' })
                                }}
                                style={{ width: 'auto' }}
                              >
                                <option value="agent">{tr.agent || 'Agent'}</option>
                                <option value="subagent">{tr.subagent || 'Subagent'}</option>
                              </select>
                              <button
                                className="field-btn danger"
                                onClick={() => {
                                  handleDeleteAgent(agent.name)
                                  setSelectedAgentIdx(null)
                                }}
                              >
                                {tr.remove}
                              </button>
                            </div>
                            <p className="field-hint">{tr.agentTypeHint}</p>
                          </div>
                          <div className="field">
                            <label className="field-label">{tr.agentDisplayName}</label>
                            <input
                              className="field-input"
                              type="text"
                              defaultValue={agent.display_name || ''}
                              onBlur={(e) => {
                                if (e.target.value !== (agent.display_name || '')) {
                                  handleSaveAgent(agent.name, { ...agent, display_name: e.target.value })
                                }
                              }}
                            />
                            <p className="field-hint">{tr.agentModeHint}</p>
                          </div>
                          <div className="field">
                            <label className="field-label">{tr.agentDescription}</label>
                            <input
                              className="field-input"
                              type="text"
                              defaultValue={agent.description || ''}
                              onBlur={(e) => {
                                if (e.target.value !== (agent.description || '')) {
                                  handleSaveAgent(agent.name, { ...agent, description: e.target.value })
                                }
                              }}
                            />
                            {isSubagent && <p className="field-hint">{tr.agentDescriptionHint}</p>}
                          </div>
                          {!isSubagent && (
                            <div className="field">
                              <label className="field-label">{tr.agentSafetyLabel}</label>
                              <select
                                className="field-select"
                                value={agent.safety || 'neutral'}
                                onChange={(e) => {
                                  handleSaveAgent(agent.name, { ...agent, safety: e.target.value })
                                }}
                              >
                                {['safe', 'neutral', 'destructive', 'smart', 'yolo'].map((s) => (
                                  <option key={s} value={s}>
                                    {s}
                                  </option>
                                ))}
                              </select>
                              <p className="field-hint">{tr.agentSafetyHint}</p>
                            </div>
                          )}
                          <div className="field">
                            <label className="field-label">{tr.modelOptional}</label>
                            <select
                              className="field-select"
                              value={agent.active_model || ''}
                              onChange={(e) => {
                                handleSaveAgent(agent.name, { ...agent, active_model: e.target.value })
                              }}
                            >
                              <option value="">{tr.none || '-- None --'}</option>
                              {(() => {
                                const local = vibeConfig.models || []
                                const localKeys = local.map((m) => m.alias || m.name)
                                const all = [...local, ...globalModels.filter((m) => !localKeys.includes(m.alias || m.name))]
                                return all.map((model) => {
                                  const key = model.alias || model.name
                                  const fromGlobal = !localKeys.includes(key)
                                  return (
                                    <option key={key + (fromGlobal ? ':g' : '')} value={key}>
                                      {model.display_name || key} ({model.provider}){fromGlobal ? ' ' + tr.fromGlobalScope : ''}
                                    </option>
                                  )
                                })
                              })()}
                            </select>
                            <p className="field-hint">{tr.agentModelHint}</p>
                          </div>
                          <div className="field">
                            <label className="field-label">{tr.systemPrompt}</label>
                            <select
                              className="field-select"
                              value={agent.system_prompt_id || ''}
                              onChange={(e) => {
                                handleSaveAgent(agent.name, { ...agent, system_prompt_id: e.target.value })
                              }}
                            >
                              <option value="">{tr.none || '-- None --'}</option>
                              {['cli', 'explore', 'tests', 'lean', 'minimal'].map((id) => (
                                <option key={'builtin-' + id} value={id}>
                                  {id} ({tr.builtinPromptLabel})
                                </option>
                              ))}
                              {vibePrompts.map((prompt) => (
                                <option key={prompt.name} value={prompt.name}>
                                  {prompt.name}
                                </option>
                              ))}
                            </select>
                            <p className="field-hint">{tr.agentSystemPromptHint}</p>
                          </div>
                          {isSubagent && (
                            <div className="field">
                              <label className="field-label">{tr.agentInstructions}</label>
                              <textarea
                                className="field-textarea"
                                rows={4}
                                defaultValue={agent.instructions || ''}
                                onBlur={(e) => {
                                  if (e.target.value !== (agent.instructions || '')) {
                                    handleSaveAgent(agent.name, { ...agent, instructions: e.target.value })
                                  }
                                }}
                                placeholder={tr.systemPromptPlaceholder}
                              />
                              <p className="field-hint">{tr.agentInstructionsHint}</p>
                            </div>
                          )}
                          <div className="field">
                            <label className="field-label">{tr.enabledTools || 'Enabled Tools'}</label>
                            <input
                              className="field-input"
                              type="text"
                              value={(agent.enabled_tools || []).join(', ')}
                              onChange={(e) => {
                                const tools = e.target.value.split(',').map(t => t.trim()).filter(Boolean)
                                handleSaveAgent(agent.name, { ...agent, enabled_tools: tools.length > 0 ? tools : undefined })
                              }}
                              placeholder={tr.toolsPlaceholder}
                            />
                          </div>
                          <div className="field">
                            <label className="field-label">{tr.disabledTools || 'Disabled Tools'}</label>
                            <input
                              className="field-input"
                              type="text"
                              value={(agent.disabled_tools || []).join(', ')}
                              onChange={(e) => {
                                const tools = e.target.value.split(',').map(t => t.trim()).filter(Boolean)
                                handleSaveAgent(agent.name, { ...agent, disabled_tools: tools.length > 0 ? tools : undefined })
                              }}
                              placeholder="tool1, tool2"
                            />
                          </div>
                          {renderToolConfigSection(agent.name)}
                        </div>
                      )
                    })() : (
                      <div className="master-detail-empty">{tr.noAgentsConfigured}</div>
                    )}
                  </div>
                </div>
              )}
            </div>
            <div className="collapsible-panel" style={{ display: globalToolsCollapsed ? 'none' : 'flex' }}>
              <div className="collapsible-header" onClick={() => setGlobalToolsCollapsed(!globalToolsCollapsed)}>
                <h3 className="settings-section-title">{tr.globalToolPermissions || 'Global Tool Permissions'}</h3>
                <span className="collapsible-arrow">{globalToolsCollapsed ? '▶' : '▼'}</span>
              </div>
              {!globalToolsCollapsed && (
                <div className="tools-list">
                  {vibeConfig.tools && Object.keys(vibeConfig.tools).length > 0 ? (
                    Object.entries(vibeConfig.tools).map(([toolName, toolPerm]) => {
                      const perm = toolPerm as VibeToolPermission
                      return (
                      <div key={toolName} className="tool-item">
                        <span className="tool-name">{toolName.replace('tools.', '')}</span>
                        <span className="tool-permission">{perm.permission || 'ask'}</span>
                      </div>
                    )})
                  ) : (
                    <div>
                      <p className="default-tools-note">{tr.defaultToolsHint}</p>
                      {DEFAULT_VIBE_TOOLS.map((t) => (
                        <div key={t} className="tool-item">
                          <span className="tool-name">{t}</span>
                          <span className="tool-permission">ask</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        )}

        {tab === 'mcp' && (
          <div className="settings-section">
            <div className="settings-section-header">
              <h3 className="settings-section-title">{tr.mcpServers}</h3>
              <button className="field-btn" onClick={handleAddMcpServer}>
                {tr.addMcpServer}
              </button>
            </div>
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : (vibeConfig.mcp_servers || []).length === 0 ? (
              <p className="empty-text">{tr.noMcpConfigured}</p>
            ) : (
              <div className="master-detail">
                <div className="master-list">
                  {vibeConfig.mcp_servers?.map((server, idx) => (
                    <div
                      key={`${server.name}-${idx}`}
                      className={`master-item ${selectedMcpIdx === idx ? 'selected' : ''}`}
                      onClick={() => setSelectedMcpIdx(idx)}
                    >
                      <span className="master-item-name">{server.name}</span>
                      <span className="master-item-type">{server.transport}</span>
                    </div>
                  ))}
                </div>
                <div className="master-detail-panel">
                  {selectedMcpIdx !== null && vibeConfig.mcp_servers?.[selectedMcpIdx] ? (() => {
                    const server = vibeConfig.mcp_servers[selectedMcpIdx]
                    const idx = selectedMcpIdx
                    const auth = server.auth || mcpLegacyAuth(server)
                    return (
                      <div>
                        <div className="field">
                          <label className="field-label">{tr.name}</label>
                          <div className="config-item-header-row">
                            <input
                              className="field-input"
                              type="text"
                              value={server.name}
                              onChange={(e) => handleUpdateMcpServer(idx, { name: e.target.value })}
                              placeholder={tr.serverName}
                              disabled={mcpJsonMode}
                              title={mcpJsonMode ? tr.mcpNameFromJson : undefined}
                            />
                            <button
                              className="field-btn danger"
                              onClick={() => {
                                handleRemoveMcpServer(idx)
                                setSelectedMcpIdx(null)
                              }}
                            >
                              {tr.remove}
                            </button>
                            <button
                              className={'field-btn' + (mcpJsonMode ? '' : ' primary')}
                              onClick={() => {
                                setMcpJsonText(JSON.stringify(server, null, 2))
                                setMcpJsonError(null)
                                setMcpJsonMode(false)
                              }}
                            >
                              {tr.mcpFormView}
                            </button>
                            <button
                              className={'field-btn' + (mcpJsonMode ? ' primary' : '')}
                              onClick={() => {
                                setMcpJsonText(JSON.stringify(server, null, 2))
                                setMcpJsonError(null)
                                setMcpJsonMode(true)
                              }}
                            >
                              {tr.mcpJsonView}
                            </button>
                          </div>
                        </div>
                        {!mcpJsonMode ? (<>
                        <div className="field">
                          <label className="field-label">{tr.transport || 'Transport'}</label>
                          <select
                            className="field-select"
                            value={server.transport}
                            onChange={(e) => {
                              const t = e.target.value as 'http' | 'streamable-http' | 'stdio'
                              if (t === 'stdio') {
                                handleUpdateMcpServer(idx, {
                                  transport: t,
                                  url: undefined,
                                  auth: undefined,
                                  headers: undefined,
                                  api_key_env: undefined,
                                  api_key_header: undefined,
                                  api_key_format: undefined,
                                })
                              } else {
                                handleUpdateMcpServer(idx, {
                                  transport: t,
                                  command: undefined,
                                  args: undefined,
                                  env: undefined,
                                  cwd: undefined,
                                  auth: server.auth || { type: 'static' },
                                })
                              }
                            }}
                          >
                            <option value="stdio">stdio</option>
                            <option value="http">http</option>
                            <option value="streamable-http">streamable-http</option>
                          </select>
                        </div>
                        {server.transport === 'http' || server.transport === 'streamable-http' ? (
                          <>
                            <div className="field">
                              <label className="field-label">URL</label>
                              <input
                                className="field-input"
                                type="text"
                                value={server.url || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { url: e.target.value })}
                                placeholder="http://localhost:3000"
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">{tr.authType}</label>
                              <select
                                className="field-select"
                                value={auth.type}
                                onChange={(e) => {
                                  if (e.target.value === 'oauth') {
                                    handleUpdateMcpServer(idx, mcpOauthUpdate(server, {}))
                                  } else {
                                    handleUpdateMcpServer(idx, mcpAuthUpdate(server, {}))
                                  }
                                }}
                              >
                                <option value="static">{tr.authStatic}</option>
                                <option value="oauth">{tr.authOauth}</option>
                              </select>
                              <p className="field-hint">{auth.type === 'oauth' ? tr.authOauthHint : tr.authStaticHint}</p>
                            </div>
                            {auth.type === 'oauth' ? (
                              <>
                                <div className="field">
                                  <label className="field-label">{tr.oauthScopes}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={(auth.scopes || []).join(', ')}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpOauthUpdate(server, { scopes: e.target.value.split(',').map((sv) => sv.trim()).filter(Boolean) }))}
                                    placeholder="read, write"
                                  />
                                  <p className="field-hint">{tr.oauthScopesHint}</p>
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.oauthClientId}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={auth.client_id || ''}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpOauthUpdate(server, { client_id: e.target.value }))}
                                    placeholder="my-mcp-client"
                                  />
                                  <p className="field-hint">{tr.oauthClientIdHint}</p>
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.oauthMetadataUrl}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={auth.client_metadata_url || ''}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpOauthUpdate(server, { client_metadata_url: e.target.value }))}
                                    placeholder="https://example.com/.well-known/oauth-authorization-server"
                                  />
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.oauthRedirectPort}</label>
                                  <input
                                    className="field-input"
                                    type="number"
                                    value={auth.redirect_port || 47823}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpOauthUpdate(server, { redirect_port: Number(e.target.value) }))}
                                    placeholder="47823"
                                  />
                                  <p className="field-hint">{tr.oauthRedirectPortHint}</p>
                                </div>
                              </>
                            ) : (
                              <>
                                <div className="field">
                                  <label className="field-label">{tr.mcpHeaders}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={JSON.stringify(auth.headers || {})}
                                    onChange={(e) => {
                                      try {
                                        handleUpdateMcpServer(idx, mcpAuthUpdate(server, { headers: JSON.parse(e.target.value) }))
                                      } catch {}
                                    }}
                                    placeholder='{ "Authorization" = "Bearer token" }'
                                  />
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.apiKeyEnv}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={auth.api_key_env || ''}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpAuthUpdate(server, { api_key_env: e.target.value }))}
                                    placeholder="MCP_API_KEY"
                                  />
                                  <p className="field-hint">{tr.apiKeyEnvHint}</p>
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.apiKeyHeader}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={auth.api_key_header || ''}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpAuthUpdate(server, { api_key_header: e.target.value }))}
                                    placeholder="Authorization"
                                  />
                                </div>
                                <div className="field">
                                  <label className="field-label">{tr.apiKeyFormat}</label>
                                  <input
                                    className="field-input"
                                    type="text"
                                    value={auth.api_key_format || ''}
                                    onChange={(e) => handleUpdateMcpServer(idx, mcpAuthUpdate(server, { api_key_format: e.target.value }))}
                                    placeholder="Bearer {token}"
                                  />
                                </div>
                              </>
                            )}
                          </>
                        ) : (
                          <>
                            <div className="field">
                              <label className="field-label">{tr.command}</label>
                              <input
                                className="field-input"
                                type="text"
                                value={server.command || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { command: e.target.value })}
                                placeholder={tr.commandPlaceholder}
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">{tr.argumentsJson}</label>
                              <input
                                className="field-input"
                                type="text"
                                value={JSON.stringify(server.args || [])}
                                onChange={(e) => {
                                  try {
                                    handleUpdateMcpServer(idx, { args: JSON.parse(e.target.value) })
                                  } catch {}
                                }}
                                placeholder={tr.argumentsPlaceholder}
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">{tr.envJson}</label>
                              <input
                                className="field-input"
                                type="text"
                                value={JSON.stringify(server.env || {})}
                                onChange={(e) => {
                                  try {
                                    handleUpdateMcpServer(idx, { env: JSON.parse(e.target.value) })
                                  } catch {}
                                }}
                                placeholder='{ "DEBUG" = "1" }'
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">{tr.mcpCwd}</label>
                              <input
                                className="field-input"
                                type="text"
                                value={server.cwd || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { cwd: e.target.value })}
                                placeholder="/path/to/dir"
                              />
                              <p className="field-hint">{tr.mcpCwdHint}</p>
                            </div>
                          </>
                        )}
                        <div className="field">
                          <label className="field-label">{tr.startupTimeout}</label>
                          <input
                            className="field-input"
                            type="number"
                            value={server.startup_timeout_sec || 10}
                            onChange={(e) => handleUpdateMcpServer(idx, { startup_timeout_sec: Number(e.target.value) })}
                          />
                          <p className="field-hint">{tr.startupTimeoutHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.toolTimeout}</label>
                          <input
                            className="field-input"
                            type="number"
                            value={server.tool_timeout_sec || 60}
                            onChange={(e) => handleUpdateMcpServer(idx, { tool_timeout_sec: Number(e.target.value) })}
                          />
                          <p className="field-hint">{tr.toolTimeoutHint}</p>
                        </div>
                        </>) : (
                          <div>
                            <textarea
                              className="field-textarea mcp-json-textarea"
                              spellCheck={false}
                              value={mcpJsonText}
                              onChange={(e) => { setMcpJsonText(e.target.value); setMcpJsonError(null) }}
                            />
                            {mcpJsonError ? (
                              <p className="field-hint" style={{ color: 'var(--danger, #e5534b)' }}>{mcpJsonError}</p>
                            ) : null}
                            <div className="config-item-header-row">
                              <button
                                className="field-btn primary"
                                onClick={() => {
                                  try {
                                    const parsed = JSON.parse(mcpJsonText) as Partial<VibeMcpServerConfig>
                                    handleUpdateMcpServer(idx, parsed)
                                    setMcpJsonMode(false)
                                  } catch (err) {
                                    setMcpJsonError(tr.mcpJsonInvalid.replace('{{error}}', err instanceof Error ? err.message : String(err)))
                                  }
                                }}
                              >
                                {tr.mcpJsonApply}
                              </button>
                            </div>
                          </div>
                        )}
                      </div>
                    )
                  })() : (
                    <div className="master-detail-empty">{tr.noMcpConfigured}</div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'tools' && (
          <div className="settings-section tools-section">
            <div className="tools-config-panel">
              <div className="settings-section-header">
                <h3 className="settings-section-title">{tr.globalToolPermissions || 'Global Tool Permissions'}</h3>
              </div>
              {!loaded ? (
                <p className="empty-text">{tr.loading}</p>
              ) : (
                renderToolConfigSection(null)
              )}
            </div>
            <div className="tools-list-panel">
              <div className="settings-section-header">
                <h3 className="settings-section-title">{tr.availableTools || 'Available Tools'}</h3>
              </div>
              <div className="tools-list">
                {vibeConfig.tools && Object.keys(vibeConfig.tools).length > 0 ? (
                  Object.entries(vibeConfig.tools).map(([toolName, toolPerm]) => {
                    const perm = toolPerm as VibeToolPermission
                    return (
                    <div key={toolName} className="tool-item">
                      <span className="tool-name">{toolName.replace('tools.', '')}</span>
                      <span className="tool-permission">{perm.permission || 'ask'}</span>
                    </div>
                  )})
                ) : (
                  <div>
                    <p className="default-tools-note">{tr.defaultToolsHint}</p>
                    {DEFAULT_VIBE_TOOLS.map((t) => (
                      <div key={t} className="tool-item">
                        <span className="tool-name">{t}</span>
                        <span className="tool-permission">ask</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        )}

        {tab === 'prompts' && (
          <div className="settings-section">
            <div className="settings-section-header">
              <h3 className="settings-section-title">{tr.prompts}</h3>
              <button className="field-btn" onClick={handleAddPrompt}>
                {tr.addPrompt}
              </button>
            </div>
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : vibePrompts.length === 0 ? (
              <p className="empty-text">{tr.noPromptsConfigured}</p>
            ) : (
              <div className="master-detail">
                <div className="master-list">
                  {vibePrompts.map((prompt, idx) => (
                    <div
                      key={prompt.name}
                      className={`master-item ${selectedPromptIdx === idx ? 'selected' : ''}`}
                      onClick={() => setSelectedPromptIdx(idx)}
                    >
                      <span className="master-item-name">{prompt.name}</span>
                    </div>
                  ))}
                </div>
                <div className="master-detail-panel">
                  {selectedPromptIdx !== null && vibePrompts[selectedPromptIdx] ? (() => {
                    const prompt = vibePrompts[selectedPromptIdx]
                    return (
                      <div>
                        <div className="field">
                          <label className="field-label">{tr.name}</label>
                          <div className="config-item-header-row">
                            <input
                              key={prompt.name}
                              className="field-input"
                              type="text"
                              defaultValue={prompt.name}
                              onBlur={(e) => {
                                if (e.target.value !== prompt.name) {
                                  handleSavePrompt(prompt.name, e.target.value, prompt.content)
                                }
                              }}
                              placeholder={tr.promptName}
                            />
                            <button
                              className="field-btn danger"
                              onClick={() => {
                                handleDeletePrompt(prompt.name)
                                setSelectedPromptIdx(null)
                              }}
                            >
                              {tr.remove}
                            </button>
                          </div>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.content}</label>
                          <textarea
                            className="field-textarea"
                            value={prompt.content}
                            onChange={(e) => handleSavePrompt(prompt.name, prompt.name, e.target.value)}
                            placeholder={tr.contentPlaceholder}
                            rows={10}
                          />
                        </div>
                      </div>
                    )
                  })() : (
                    <div className="master-detail-empty">{tr.noPromptsConfigured}</div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'skills' && (
          <div className="settings-section">
            <div className="settings-section-header">
              <h3 className="settings-section-title">{tr.skills}</h3>
              <div className="config-item-header-row">
                <input
                  className="field-input skill-new-name"
                  type="text"
                  value={newSkillName}
                  placeholder={tr.newSkillName}
                  onChange={(e) => setNewSkillName(e.target.value)}
                />
                <button className="field-btn primary" onClick={handleCreateSkill}>
                  {tr.createSkill}
                </button>
                <button className="field-btn" onClick={handleImportSkillFolder}>
                  {tr.importSkillFolder}
                </button>
                <button className="field-btn" onClick={handleImportSkillZip}>
                  {tr.importSkillZip}
                </button>
                <button className="field-btn" onClick={handleOpenSkillsFolder}>
                  {tr.openSkillsFolder}
                </button>
              </div>
            </div>
            {skillMsg && <p className="field-hint">{skillMsg}</p>}
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : vibeSkills.length === 0 ? (
              <p className="empty-text">{tr.noVibeSkillsConfigured}</p>
            ) : (
              <div className="master-detail">
                <div className="master-list">
                  {vibeSkills.map((skill, sIdx) => (
                    <div
                      key={skill.name}
                      className={'master-item' + (selectedSkillIdx === sIdx ? ' selected' : '')}
                      onClick={() => setSelectedSkillIdx(sIdx)}
                    >
                      <span className="master-item-name">{skill.name}</span>
                      <span className="master-item-type">{skill.files?.length ?? 0}</span>
                    </div>
                  ))}
                </div>
                <div className="master-detail-panel">
                  {selectedSkillIdx !== null && vibeSkills[selectedSkillIdx] ? (() => {
                    const skill = vibeSkills[selectedSkillIdx]
                    return (
                      <div className="skill-detail">
                        <div className="config-item-header-row">
                          <h4 className="config-item-name">{skill.name}</h4>
                          <button
                            className="field-btn"
                            onClick={() => { if (skill.dir) window.api.shell.openPath(skill.dir).catch(console.error) }}
                          >
                            {tr.previewOpenExternally}
                          </button>
                          <button className="field-btn danger" onClick={() => handleDeleteSkill(skill)}>
                            {tr.deleteSkill}
                          </button>
                        </div>
                        <div className="skill-detail-body">
                          <div className="dirtree-col">
                            <div className="config-item-header-row">
                              <input
                                className="field-input"
                                type="text"
                                value={newSkillFileName}
                                placeholder={tr.newSkillFileName}
                                onChange={(e) => setNewSkillFileName(e.target.value)}
                              />
                              <button className="field-btn" onClick={() => handleAddSkillFile(skill)}>
                                {tr.addFile}
                              </button>
                            </div>
                            {skill.dir ? (
                              <DirTree
                                rootDir={skill.dir}
                                remoteId={vibeRemoteId}
                                selectedFull={selectedSkillFile?.full ?? null}
                                onSelect={(sel) => {
                                  if (!sel.isDir) setSelectedSkillFile({ path: sel.rel, full: sel.full, isDir: false })
                                }}
                                onDeleteFile={(sel) => handleDeleteSkillFileBtn(skill, sel.rel)}
                                deleteTitle={tr.deleteFile}
                                refreshKey={skillTreeRev}
                              />
                            ) : (
                              <p className="empty-text">{tr.skillFilesHint}</p>
                            )}
                          </div>
                          <div className="vibe-editor-col">
                            {selectedSkillFile && !selectedSkillFile.isDir && skillFileReady === selectedSkillFile.full ? (
                              <>
                                <div className="config-item-header-row">
                                  <span className="field-label">{selectedSkillFile.path}</span>
                                  <button className="field-btn primary" onClick={() => handleSaveSkillFile(skill)}>
                                    {tr.saveFile}
                                  </button>
                                </div>
                                <div className="vibe-editor-box">
                                  <CodeMirrorPane
                                    path={selectedSkillFile.full}
                                    content={skillFileDraft}
                                    editable
                                    registerDoc={registerSkillDoc}
                                    onSave={() => { void handleSaveSkillFile(skill) }}
                                  />
                                </div>
                              </>
                            ) : (
                              <p className="empty-text">{tr.selectSkillFileHint}</p>
                            )}
                          </div>
                        </div>
                      </div>
                    )
                  })() : (
                    <div className="master-detail-empty">{tr.selectSkillHint}</div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'providers' && (
          <div className="settings-section">
            <div className="settings-section-header">
              <h3 className="settings-section-title">{tr.providers}</h3>
              <button className="field-btn" onClick={handleAddProvider}>
                {tr.addProvider}
              </button>
            </div>
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : (vibeConfig.providers || []).length === 0 ? (
              <p className="empty-text">{tr.noProvidersConfigured}</p>
            ) : (
              <div className="master-detail">
                <div className="master-list">
                  {vibeConfig.providers?.map((provider, idx) => (
                    <div
                      key={`${provider.name}-${idx}`}
                      className={`master-item ${selectedProviderIdx === idx ? 'selected' : ''}`}
                      onClick={() => setSelectedProviderIdx(idx)}
                    >
                      <span className="master-item-name">{provider.name}</span>
                      <span className="master-item-type">{provider.backend || ''}</span>
                    </div>
                  ))}
                </div>
                <div className="master-detail-panel">
                  {selectedProviderIdx !== null && vibeConfig.providers?.[selectedProviderIdx] ? (() => {
                    const provider = vibeConfig.providers[selectedProviderIdx]
                    const idx = selectedProviderIdx
                    return (
                      <div>
                        <div className="field">
                          <label className="field-label">{tr.name}</label>
                          <div className="config-item-header-row">
                            <input
                              className="field-input"
                              type="text"
                              value={provider.name}
                              onChange={(e) => handleUpdateProvider(idx, { name: e.target.value })}
                              placeholder={tr.providerName}
                            />
                            <label className="checkbox-label">
                              <input
                                type="checkbox"
                                checked={provider.enabled ?? true}
                                onChange={(e) => handleUpdateProvider(idx, { enabled: e.target.checked })}
                              />
                              {tr.enabled}
                            </label>
                            <button
                              className="field-btn danger"
                              onClick={() => {
                                handleRemoveProvider(idx)
                                setSelectedProviderIdx(null)
                              }}
                            >
                              {tr.remove}
                            </button>
                          </div>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.baseUrl || 'Base URL'}</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.api_base || ''}
                            onChange={(e) => handleUpdateProvider(idx, { api_base: e.target.value })}
                            placeholder="https://api.mistral.ai/v1"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.apiKeyEnv}</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.api_key_env_var || ''}
                            onChange={(e) => handleUpdateProvider(idx, { api_key_env_var: e.target.value })}
                            placeholder="MISTRAL_API_KEY"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.apiKey || 'API Key'}</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.api_key || ''}
                            onChange={(e) => handleUpdateProvider(idx, { api_key: e.target.value })}
                            placeholder="sk-..."
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">API Style</label>
                          <select
                            className="field-select"
                            value={provider.api_style || 'openai'}
                            onChange={(e) => handleUpdateProvider(idx, { api_style: e.target.value })}
                          >
                            <option value="openai">OpenAI</option>
                            <option value="anthropic">Anthropic</option>
                            <option value="google">Google</option>
                            <option value="azure">Azure</option>
                          </select>
                        </div>
                        <div className="field">
                          <label className="field-label">Backend</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.backend || ''}
                            onChange={(e) => handleUpdateProvider(idx, { backend: e.target.value })}
                            placeholder="mistral"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">Reasoning Field Name</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.reasoning_field_name || ''}
                            onChange={(e) => handleUpdateProvider(idx, { reasoning_field_name: e.target.value })}
                            placeholder="reasoning_content"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">Project ID</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.project_id || ''}
                            onChange={(e) => handleUpdateProvider(idx, { project_id: e.target.value })}
                            placeholder=""
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">Region</label>
                          <input
                            className="field-input"
                            type="text"
                            value={provider.region || ''}
                            onChange={(e) => handleUpdateProvider(idx, { region: e.target.value })}
                            placeholder=""
                          />
                        </div>
                      </div>
                    )
                  })() : (
                    <div className="master-detail-empty">{tr.noProvidersConfigured}</div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'models' && (
          <div className="settings-section">
            <div className="settings-section-header">
              <h3 className="settings-section-title">{tr.models}</h3>
              <button className="field-btn" onClick={handleAddModel}>
                {tr.addModel}
              </button>
            </div>
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : (vibeConfig.models || []).length === 0 ? (
              <p className="empty-text">{tr.noModelsConfigured}</p>
            ) : (
              <div className="master-detail">
                <div className="master-list">
                  {vibeConfig.models?.map((model, idx) => (
                    <div
                      key={`${model.name}-${idx}`}
                      className={`master-item ${selectedModelIdx === idx ? 'selected' : ''}`}
                      onClick={() => setSelectedModelIdx(idx)}
                    >
                      <span className="master-item-name">{model.name}</span>
                      <span className="master-item-type">{model.provider}</span>
                    </div>
                  ))}
                </div>
                <div className="master-detail-panel">
                  {selectedModelIdx !== null && vibeConfig.models?.[selectedModelIdx] ? (() => {
                    const model = vibeConfig.models[selectedModelIdx]
                    const idx = selectedModelIdx
                    return (
                      <div>
                        <div className="field">
                          <label className="field-label">{tr.name}</label>
                          <div className="config-item-header-row">
                            <input
                              className="field-input"
                              type="text"
                              value={model.name}
                              onChange={(e) => handleUpdateModel(idx, { name: e.target.value })}
                              placeholder={tr.modelName || 'Model name'}
                            />
                            <label className="checkbox-label">
                              <input
                                type="checkbox"
                                checked={model.enabled ?? true}
                                onChange={(e) => handleUpdateModel(idx, { enabled: e.target.checked })}
                              />
                              {tr.enabled}
                            </label>
                            <label className="checkbox-label">
                              <input
                                type="checkbox"
                                checked={model.supports_images ?? false}
                                onChange={(e) => handleUpdateModel(idx, { supports_images: e.target.checked })}
                              />
                              {tr.supportsImages}
                            </label>
                            <button
                              className="field-btn danger"
                              onClick={() => {
                                handleRemoveModel(idx)
                                setSelectedModelIdx(null)
                              }}
                            >
                              {tr.remove}
                            </button>
                          </div>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.modelProvider || 'Provider'}</label>
                          <select
                            className="field-select"
                            value={model.provider}
                            onChange={(e) => handleUpdateModel(idx, { provider: e.target.value })}
                          >
                            {(vibeConfig.providers || []).map((p) => (
                              <option key={p.name} value={p.name}>{p.name}</option>
                            ))}
                          </select>
                        </div>
                        <div className="field">
                          <label className="field-label">Alias</label>
                          <input
                            className="field-input"
                            type="text"
                            value={model.alias || ''}
                            onChange={(e) => handleUpdateModel(idx, { alias: e.target.value })}
                            placeholder="devstral-2"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.displayName}</label>
                          <input
                            className="field-input"
                            type="text"
                            value={model.display_name || ''}
                            onChange={(e) => handleUpdateModel(idx, { display_name: e.target.value })}
                            placeholder="Devstral 2"
                          />
                          <p className="field-hint">{tr.displayNameHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.temperature || 'Temperature'}</label>
                          <input
                            className="field-input"
                            type="number"
                            step="0.1"
                            min="0"
                            max="2"
                            value={model.temperature ?? 0.2}
                            onChange={(e) => handleUpdateModel(idx, { temperature: Number(e.target.value) })}
                          />
                          <p className="field-hint">{tr.temperatureHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.thinking || 'Thinking'}</label>
                          <select
                            className="field-select"
                            value={model.thinking || 'off'}
                            onChange={(e) => handleUpdateModel(idx, { thinking: e.target.value as 'off' | 'low' | 'medium' | 'high' | 'max' })}
                          >
                            <option value="off">{tr.off || 'Off'}</option>
                            <option value="low">{tr.low || 'Low'}</option>
                            <option value="medium">{tr.medium || 'Medium'}</option>
                            <option value="high">{tr.high || 'High'}</option>
                            <option value="max">{tr.max}</option>
                          </select>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.inputPrice || 'Input Price ($/M)'}</label>
                          <input
                            className="field-input"
                            type="number"
                            step="0.01"
                            value={model.input_price ?? 0}
                            onChange={(e) => handleUpdateModel(idx, { input_price: Number(e.target.value) })}
                            placeholder="0.0"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.outputPrice || 'Output Price ($/M)'}</label>
                          <input
                            className="field-input"
                            type="number"
                            step="0.01"
                            value={model.output_price ?? 0}
                            onChange={(e) => handleUpdateModel(idx, { output_price: Number(e.target.value) })}
                            placeholder="0.0"
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.cachedInputPrice}</label>
                          <input
                            className="field-input"
                            type="number"
                            step="0.01"
                            placeholder="—"
                            value={model.cached_input_price ?? ''}
                            onChange={(e) =>
                              handleUpdateModel(idx, {
                                cached_input_price:
                                  e.target.value === '' ? null : Number(e.target.value),
                              })
                            }
                          />
                          <p className="field-hint">{tr.cachedInputPriceHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.topP}</label>
                          <input
                            className="field-input"
                            type="number"
                            step="0.01"
                            min="0"
                            max="1"
                            value={model.top_p ?? 1}
                            onChange={(e) => handleUpdateModel(idx, { top_p: Number(e.target.value) })}
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.maxOutputTokens}</label>
                          <input
                            className="field-input"
                            type="number"
                            placeholder="0"
                            value={model.max_output_tokens ?? 0}
                            onChange={(e) => handleUpdateModel(idx, { max_output_tokens: Number(e.target.value) })}
                          />
                          <p className="field-hint">{tr.maxOutputTokensHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.thinkingLevels}</label>
                          <input
                            className="field-input"
                            value={(model.thinking_levels ?? []).join(', ')}
                            onChange={(e) => handleUpdateModel(idx, { thinking_levels: e.target.value.split(',').map((v) => v.trim()).filter(Boolean) })}
                            placeholder="off, low, medium, high"
                          />
                          <p className="field-hint">{tr.thinkingLevelsHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.maxContextLength}</label>
                          <input
                            className="field-input"
                            type="number"
                            placeholder="0"
                            value={model.max_context_length ?? 0}
                            onChange={(e) => handleUpdateModel(idx, { max_context_length: Number(e.target.value) })}
                          />
                          <p className="field-hint">{tr.maxContextHint}</p>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.autoCompactThreshold || 'Auto Compact Threshold'}</label>
                          <input
                            className="field-input"
                            type="number"
                            value={model.auto_compact_threshold ?? 200000}
                            onChange={(e) => handleUpdateModel(idx, { auto_compact_threshold: Number(e.target.value) })}
                            placeholder="200000"
                          />
                          <p className="field-hint">{tr.autoCompactHint}</p>
                        </div>
                      </div>
                    )
                  })() : (
                    <div className="master-detail-empty">{tr.noModelsConfigured}</div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}
        {tab === 'raw' && (
          <div className="settings-section raw-config-section">
            <div className="dirtree-col">
              <button
                className={'cfg-file-item' + (selectedCfgFile === null ? ' active' : '')}
                onClick={() => void loadConfigFile(null)}
              >
                config.toml
              </button>
              {vibeCfgDir ? (
                <DirTree
                  rootDir={vibeCfgDir}
                  remoteId={vibeRemoteId}
                  selectedFull={selectedCfgFileFull}
                  onSelect={(sel) => {
                    if (sel.isDir) return
                    if (sel.rel === 'config.toml') void loadConfigFile(null)
                    else void loadConfigFile(sel.rel, sel.full)
                  }}
                  refreshKey={scope + '|' + projectCwd}
                />
              ) : null}
            </div>
            <div className="raw-editor-col">
              <div className="field">
                <label className="field-label">{selectedCfgFile === null ? tr.rawConfig : selectedCfgFile}</label>
                <p className="field-hint">{selectedCfgFile === null ? tr.rawTomlHint : tr.rawFileHint}</p>
                {rawReadyFile !== null && ((selectedCfgFile === null && rawReadyFile === '__config__') || (selectedCfgFile !== null && rawReadyFile === selectedCfgFileFull)) ? (
                  <div className="vibe-editor-box">
                    <CodeMirrorPane
                      path={selectedCfgFile === null ? 'config.toml' : selectedCfgFileFull ?? selectedCfgFile}
                      content={rawToml ?? ''}
                      editable
                      registerDoc={registerRawDoc}
                      onSave={() => { void handleSaveRawToml() }}
                    />
                  </div>
                ) : null}
              </div>
              {rawTomlError && (
                <div className="raw-toml-error">{rawTomlError}</div>
              )}
              <div className="raw-toml-actions">
                <button className="save-btn" onClick={handleSaveRawToml}>
                  {rawTomlSaved ? tr.rawTomlSaved : tr.rawTomlSave}
                </button>
                <button
                  className="save-btn raw-reload-btn"
                  onClick={() => {
                    if (selectedCfgFile === null) void loadConfigFile(null)
                    else void loadConfigFile(selectedCfgFile, selectedCfgFileFull)
                  }}
                >
                  {tr.reload}
                </button>
              </div>
            </div>
          </div>
        )}

        {tab === 'serve' && !web && (
          <div className="settings-section">
            <p className="field-hint">{tr.serveIntro}</p>
            <div className="field">
              <label className="field-label">{tr.serveStatusLabel}</label>
              <div className="field-row">
                <button className="field-btn" onClick={() => void handleServeToggle()}>
                  {serveStatus?.running ? tr.serveStop : tr.serveStart}
                </button>
                {serveStatus && (
                  <span className={`serve-status ${serveStatus.running ? 'ok' : ''}`}>
                    {serveStatus.running ? `${tr.serveRunning} — ${serveStatus.url}` : tr.serveStopped}
                  </span>
                )}
                {serveError && <div className="git-error">{serveError}</div>}
              </div>
            {serveStatus?.running && (serveStatus.lanUrls?.length ?? 0) > 0 && (
              <div className="serve-lan-urls">
                <div className="serve-lan-label">{tr.serveLanUrls}</div>
                {serveStatus.lanUrls.map((u) => (
                  <div key={u} className="serve-lan-url">{u}</div>
                ))}
              </div>
            )}
            </div>
            <div className="field">
              <label className="field-label">{tr.servePort}</label>
              <div className="field-row">
                <input
                  className="field-input"
                  type="number"
                  style={{ width: 110 }}
                  value={local.serve?.port ?? defaultConfig.serve.port}
                  onChange={(e) =>
                    setLocal((p) => ({
                      ...p,
                      serve: {
                        ...(p.serve ?? defaultConfig.serve),
                        port: parseInt(e.target.value, 10) || 0,
                      },
                    }))
                  }
                />
                <button className="field-btn" onClick={() => void handleServeApply()}>
                  {tr.saveLabel}
                </button>
              </div>
              <p className="field-hint">{tr.servePortHint}</p>
            </div>
            <div className="field">
              <label className="field-label">{tr.serveAutostart}</label>
              <div className="field-row">
                <button
                  className={`field-btn ${(local.serve?.enabled ?? false) ? 'active' : ''}`}
                  onClick={() =>
                    setLocal((p) => ({
                      ...p,
                      serve: {
                        ...(p.serve ?? defaultConfig.serve),
                        enabled: !(p.serve?.enabled ?? false),
                      },
                    }))
                  }
                >
                  {(local.serve?.enabled ?? false) ? tr.serveOn : tr.serveOff}
                </button>
              </div>
              <p className="field-hint">{tr.serveAutostartHint}</p>
            </div>
            <div className="field">
              <label className="field-label">{tr.serveCredentials}</label>
              <p className="field-hint">{tr.serveCredentialsHint}</p>
              <div className="field-row">
                <input
                  className="field-input"
                  type="text"
                  placeholder={tr.serveUsername}
                  value={serveUsername}
                  onChange={(e) => setServeUsername(e.target.value)}
                />
                <input
                  className="field-input"
                  type="password"
                  placeholder={tr.serveNewPassword}
                  value={servePassword}
                  onChange={(e) => setServePassword(e.target.value)}
                />
                <button className="field-btn" onClick={() => void handleServeCredentials()}>
                  {serveCredsSaved ? tr.serveCredentialsSaved : tr.saveLabel}
                </button>
              </div>
            </div>
            <div className="field">
              <label className="field-label">{tr.serveCert}</label>
              <p className="field-hint">{tr.serveCertHint}</p>
              <div className="field-row">
                <input
                  className="field-input"
                  type="text"
                  placeholder="cert.pem"
                  value={local.serve?.certPath ?? ''}
                  onChange={(e) =>
                    setLocal((p) => ({
                      ...p,
                      serve: { ...(p.serve ?? defaultConfig.serve), certPath: e.target.value },
                    }))
                  }
                />
                <button className="field-btn" onClick={() => void handleServeCertBrowse('certPath')}>
                  {tr.browse}
                </button>
              </div>
              <div className="field-row">
                <input
                  className="field-input"
                  type="text"
                  placeholder="key.pem"
                  value={local.serve?.keyPath ?? ''}
                  onChange={(e) =>
                    setLocal((p) => ({
                      ...p,
                      serve: { ...(p.serve ?? defaultConfig.serve), keyPath: e.target.value },
                    }))
                  }
                />
                <button className="field-btn" onClick={() => void handleServeCertBrowse('keyPath')}>
                  {tr.browse}
                </button>
              </div>
              <div className="field-row">
                <button className="field-btn" onClick={() => void handleServeApply()}>
                  {tr.saveLabel}
                </button>
                <button
                  className="field-btn"
                  onClick={() => void window.api.serve.regenCert().catch(console.error)}
                >
                  {tr.serveRegenCert}
                </button>
              </div>
            </div>
          </div>
        )}
        {tab === 'remote' && (
          <div className="settings-section">
            <p className="field-hint">{tr.remoteIntro}</p>
            {remoteDraft === null ? (
              <>
                <div className="field">
                  <button
                    className="field-btn"
                    onClick={() => {
                      setRemoteTestResult(null)
                      setRemoteInstallMsg(null)
                      setRemoteDraft({
                        id: '',
                        name: '',
                        enabled: true,
                        sshCmd: '',
                        locations: [],
                        installCommand: 'uv tool install mistral-vibe',
                      })
                    }}
                  >
                    + {tr.remoteAdd}
                  </button>
                </div>
                {(local.remoteConnections ?? []).length === 0 && (
                  <p className="field-hint">{tr.remoteNoConnections}</p>
                )}
                {(local.remoteConnections ?? []).map((conn) => (
                  <div key={conn.id} className="remote-conn-row">
                    <span className={`remote-dot ${remoteStatuses[conn.id]?.ok ? 'ok' : 'down'}`} />
                    <span className="remote-conn-name">{conn.name}</span>
                    <span className="remote-conn-cmd" title={conn.sshCmd}>
                      {conn.sshCmd}
                    </span>
                    <button
                      className="field-btn"
                      onClick={() => {
                        setRemoteTestResult(null)
                        setRemoteInstallMsg(null)
                        setRemoteDraft({ ...conn, locations: [...conn.locations] })
                      }}
                    >
                      {tr.settings}
                    </button>
                    <button className="field-btn" onClick={() => void handleRemoteDelete(conn.id)}>
                      {tr.deleteLabel}
                    </button>
                  </div>
                ))}
                <div className="remote-cli-updates">
                  <label className="field-label">{tr.updatesRemoteCliTitle}</label>
                  {(local.remoteConnections ?? []).filter((c) => c.enabled).length === 0 && (
                    <p className="field-hint">{tr.remoteNoConnections}</p>
                  )}
                  {(local.remoteConnections ?? []).filter((c) => c.enabled).map((conn) => {
                    const info = remoteCli[conn.id]
                    return (
                      <div key={conn.id} className="updates-row">
                        <div className="updates-info">
                          <span className="updates-name">{conn.name}</span>
                          <span className="updates-versions">
                            {info === 'checking'
                              ? tr.updatesChecking
                              : info
                                ? tr.updatesCurrentLabel.replace('{{version}}', info.currentVersion ?? '?') +
                                  ' · ' +
                                  tr.updatesLatestLabel.replace('{{version}}', info.latestVersion ?? '?')
                                : tr.updatesNotChecked}
                          </span>
                        </div>
                        <div className="updates-actions">
                          <button
                            className="field-btn"
                            disabled={updatesBusy}
                            onClick={() => void handleCheckRemoteCli(conn.id)}
                          >
                            {tr.updatesCheckButton}
                          </button>
                          <button
                            className="action-btn"
                            disabled={
                              updatesBusy || !info || info === 'checking' || !info.updateAvailable
                            }
                            onClick={() => void handleUpdateCliNow(conn.id)}
                          >
                            {tr.updatesUpdateButton}
                          </button>
                        </div>
                      </div>
                    )
                  })}
                </div>
              </>
            ) : (
              <div className="remote-editor">
                <div className="field">
                  <label className="field-label">{tr.remoteName}</label>
                  <input
                    className="field-input"
                    type="text"
                    value={remoteDraft.name}
                    onChange={(e) => setRemoteDraft((p) => (p ? { ...p, name: e.target.value } : p))}
                  />
                </div>
                <div className="field">
                  <label className="field-label">{tr.remoteSshCmd}</label>
                  <input
                    className="field-input"
                    type="text"
                    placeholder="ssh user@host"
                    value={remoteDraft.sshCmd}
                    onChange={(e) => setRemoteDraft((p) => (p ? { ...p, sshCmd: e.target.value } : p))}
                  />
                  <p className="field-hint">{tr.remoteSshCmdHint}</p>
                </div>
                <div className="field">
                  <label className="field-label">{tr.remoteEnabled}</label>
                  <button
                    className={`field-btn ${remoteDraft.enabled ? 'active' : ''}`}
                    onClick={() => setRemoteDraft((p) => (p ? { ...p, enabled: !p.enabled } : p))}
                  >
                    {remoteDraft.enabled ? tr.serveOn : tr.serveOff}
                  </button>
                </div>
                <div className="field">
                  <label className="field-label">{tr.remoteLocations}</label>
                  <p className="field-hint">{tr.remoteLocationHint}</p>
                  {remoteDraft.locations.map((loc) => (
                    <div key={loc} className="field-row">
                      <input className="field-input" type="text" readOnly value={loc} />
                      <button
                        className="field-btn"
                        onClick={() =>
                          setRemoteDraft((p) =>
                            p ? { ...p, locations: p.locations.filter((l) => l !== loc) } : p,
                          )
                        }
                      >
                        ✕
                      </button>
                    </div>
                  ))}
                  <div className="field-row">
                    <input
                      className="field-input"
                      type="text"
                      placeholder="/home/user/project"
                      value={remoteNewLocation}
                      onChange={(e) => setRemoteNewLocation(e.target.value)}
                    />
                    <button
                      className="field-btn"
                      onClick={() => {
                        const loc = remoteNewLocation.trim()
                        if (!loc) return
                        setRemoteDraft((p) =>
                          p && !p.locations.includes(loc)
                            ? { ...p, locations: [...p.locations, loc] }
                            : p,
                        )
                        setRemoteNewLocation('')
                      }}
                    >
                      + {tr.remoteAddLocation}
                    </button>
                  </div>
                </div>
                <div className="field">
                  <div className="field-row">
                    <button
                      className="field-btn"
                      disabled={remoteTesting}
                      onClick={() => void handleRemoteTest()}
                    >
                      {remoteTesting ? '…' : tr.remoteTest}
                    </button>
                    {!web && remoteTestResult?.reachable && !remoteTestResult.acpInstalled && (
                      <button
                        className="field-btn"
                        disabled={remoteInstalling}
                        onClick={() => void handleRemoteInstall()}
                      >
                        {remoteInstalling ? '…' : tr.remoteInstallAcp}
                      </button>
                    )}
                  </div>
                  {remoteTesting && <p className="field-hint">…</p>}
                  {remoteTestResult && (
                    <p
                      className={`field-hint ${
                        remoteTestResult.reachable ? 'remote-test-ok' : 'remote-test-fail'
                      }`}
                    >
                      {remoteTestResult.reachable
                        ? remoteTestResult.acpInstalled
                          ? t(tr, 'remoteTestOk', { version: remoteTestResult.version ?? '?' })
                          : tr.remoteTestNoAcp
                        : t(tr, 'remoteTestFail', { error: remoteTestResult.error ?? '' })}
                    </p>
                  )}
                  {remoteInstallMsg && <p className="field-hint">{remoteInstallMsg}</p>}
                </div>
                <div className="field-row">
                  <button className="field-btn" onClick={() => void handleRemoteSave()}>
                    {tr.remoteSave}
                  </button>
                  <button className="field-btn" onClick={() => setRemoteDraft(null)}>
                    {tr.cancelLabel}
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      <div className="settings-footer">
        <button className="save-btn" onClick={handleSave}>
          {saved ? tr.saved : tr.saveSettings}
        </button>
      </div>
    </div>
  )
}
