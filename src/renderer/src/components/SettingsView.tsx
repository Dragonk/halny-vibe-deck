import React, { useState, useEffect, useCallback } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import type { Language } from '../i18n'

type Tab = 'general' | 'acp' | 'sandbox' | 'agents' | 'mcp' | 'tools' | 'prompts' | 'skills' | 'providers' | 'models'

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
  active_model?: string
  system_prompt_id?: string
  disabled_tools?: string[]
  enabled_tools?: string[]
  tools?: Record<string, VibeToolPermission>
}

interface VibeMcpServerConfig {
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
  temperature?: number
  input_price?: number
  output_price?: number
  thinking?: 'off' | 'low' | 'medium' | 'high'
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
}

export default function SettingsView(): React.JSX.Element {
  const { config, setConfig, setWorkingDirectory, setSandboxEnabled, setCurrentView, acpStatus } = useStore()
  const tr = useTranslation(config.language)
  const [tab, setTab] = useState<Tab>('general')
  const [local, setLocal] = useState({ ...config })
  const [saved, setSaved] = useState(false)
  const [sessionsLimitFocused, setSessionsLimitFocused] = useState(false)

  const [vibeConfig, setVibeConfig] = useState<VibeConfig>({})
  const [vibeAgents, setVibeAgents] = useState<VibeAgentConfig[]>([])
  const [vibePrompts, setVibePrompts] = useState<VibePrompt[]>([])
  const [vibeSkills, setVibeSkills] = useState<VibeSkill[]>([])
  const [loaded, setLoaded] = useState(false)
  const [selectedAgentIdx, setSelectedAgentIdx] = useState<number | null>(null)
  const [selectedMcpIdx, setSelectedMcpIdx] = useState<number | null>(null)
  const [selectedPromptIdx, setSelectedPromptIdx] = useState<number | null>(null)
  const [selectedProviderIdx, setSelectedProviderIdx] = useState<number | null>(null)
  const [selectedModelIdx, setSelectedModelIdx] = useState<number | null>(null)
  const [globalToolsCollapsed, setGlobalToolsCollapsed] = useState(false)

  useEffect(() => {
    setLocal({ ...config })
  }, [config])

  const loadVibeData = useCallback(async (): Promise<void> => {
    const [cfg, agents, prompts, skills] = await Promise.all([
      window.api.vibe.getConfig(),
      window.api.vibe.getAgents(),
      window.api.vibe.getPrompts(),
      window.api.vibe.getSkills()
    ])
    setVibeConfig(cfg)
    setVibeAgents(agents)
    setVibePrompts(prompts)
    setVibeSkills(skills)
    setLoaded(true)
  }, [])

  useEffect(() => {
    loadVibeData()
  }, [loadVibeData])

  const handleSave = async (): Promise<void> => {
    await window.api.config.save(local)
    setConfig(local)
    setWorkingDirectory(local.workingDirectory)
    setSandboxEnabled(local.sandboxEnabled)
    await window.api.vibe.saveConfig(vibeConfig)
    
    if (tab !== 'general' && tab !== 'acp' && acpStatus === 'connected') {
      await window.api.acp.stop()
      await new Promise(resolve => setTimeout(resolve, 1000))
      if (local.acpPath) {
        await window.api.acp.start(local.acpPath, '', '')
      }
    }
    
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const handleAcpPathBrowse = async (): Promise<void> => {
    const file = await window.api.dialog.openFile()
    if (file) setLocal((prev) => ({ ...prev, acpPath: file }))
  }

  const handleStartAcp = async (): Promise<void> => {
    const { setAcpStatus, setAcpError, currentModel } = useStore.getState()
    if (!local.acpPath) {
      setAcpError('ACP path is not set.')
      return
    }
    setAcpStatus('connecting')
    try {
      await window.api.acp.start(local.acpPath, '', currentModel || '')
      setCurrentView('chat')
    } catch (err) {
      setAcpStatus('error')
      setAcpError((err as Error).message)
    }
  }

  const handleOpenSkillsFolder = async (): Promise<void> => {
    const dir = await window.api.vibe.getDir()
    const skillsDir = `${dir}/skills`
    await window.api.shell.openPath(skillsDir)
  }

  const handleRenameAgent = async (oldName: string, newName: string, agentConfig: VibeAgentConfig): Promise<void> => {
    if (newName && newName !== oldName) {
      await window.api.vibe.deleteAgent(oldName)
      const { name: _, ...configWithoutName } = agentConfig
      await window.api.vibe.saveAgent(newName, { ...configWithoutName, name: newName })
      await loadVibeData()
    }
  }

  const handleSaveAgent = async (name: string, agentConfig: VibeAgentConfig): Promise<void> => {
    await window.api.vibe.saveAgent(name, agentConfig)
    await loadVibeData()
  }

  const handleDeleteAgent = async (name: string): Promise<void> => {
    await window.api.vibe.deleteAgent(name)
    await loadVibeData()
  }

  const handleAddAgent = async (): Promise<void> => {
    const name = `agent-${Date.now()}`
    const newAgent: VibeAgentConfig = {
      name,
      agent_type: 'agent',
      active_model: '',
      system_prompt_id: '',
      disabled_tools: [],
      enabled_tools: []
    }
    await window.api.vibe.saveAgent(name, newAgent)
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
      await window.api.vibe.deletePrompt(oldName)
    }
    await window.api.vibe.savePrompt(newName, content)
    await loadVibeData()
  }

  const handleDeletePrompt = async (name: string): Promise<void> => {
    await window.api.vibe.deletePrompt(name)
    await loadVibeData()
  }

  const handleAddPrompt = async (): Promise<void> => {
    const name = `prompt-${Date.now()}`
    await window.api.vibe.savePrompt(name, '# New Prompt\n\nWrite your prompt here with {{variables}}.')
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
            <p className="empty-text">{tr.noToolsConfigured}</p>
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
        <h2 className="settings-title">{tr.settings}</h2>
        <div className="settings-tabs">
          <button
            className={`settings-tab ${tab === 'general' ? 'active' : ''}`}
            onClick={() => setTab('general')}
          >
            {tr.general}
          </button>
          <button
            className={`settings-tab ${tab === 'acp' ? 'active' : ''}`}
            onClick={() => setTab('acp')}
          >
            {tr.acpAgent}
          </button>
          <button
            className={`settings-tab ${tab === 'sandbox' ? 'active' : ''}`}
            onClick={() => setTab('sandbox')}
          >
            {tr.sandbox}
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

        {tab === 'acp' && (
          <div className="settings-section">
            <div className="field">
              <label className="field-label">{tr.vibeAcpPath}</label>
              <p className="field-hint" dangerouslySetInnerHTML={{ __html: tr.vibeAcpHint.replace('pip install mistral-vibe', '<code>pip install mistral-vibe</code>') }} />
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

        {tab === 'sandbox' && (
          <div className="settings-section">
            <div className="field">
              <label className="field-label checkbox-label">
                <input
                  type="checkbox"
                  checked={local.sandboxEnabled}
                  onChange={(e) => setLocal((p) => ({ ...p, sandboxEnabled: e.target.checked }))}
                />
                {tr.enableSandbox}
              </label>
              <p className="field-hint">
                {tr.sandboxHint}
              </p>
            </div>

            {local.sandboxEnabled && (
              <>
                <div className="field">
                  <label className="field-label">{tr.networkAccess}</label>
                  <select
                    className="field-select"
                    value={local.sandbox.network}
                    onChange={(e) =>
                      setLocal((p) => ({
                        ...p,
                        sandbox: {
                          ...p.sandbox,
                          network: e.target.value as 'none' | 'whitelist' | 'unrestricted'
                        }
                      }))
                    }
                  >
                    <option value="none">{tr.none}</option>
                    <option value="whitelist">{tr.whitelist}</option>
                    <option value="unrestricted">{tr.unrestricted}</option>
                  </select>
                </div>

                <div className="field">
                  <label className="field-label">{tr.timeoutSeconds}</label>
                  <input
                    className="field-input"
                    type="number"
                    value={local.sandbox.timeoutSeconds}
                    onChange={(e) =>
                      setLocal((p) => ({
                        ...p,
                        sandbox: { ...p.sandbox, timeoutSeconds: Number(e.target.value) }
                      }))
                    }
                  />
                </div>

                <div className="field">
                  <label className="field-label">{tr.memoryLimitMb}</label>
                  <input
                    className="field-input"
                    type="number"
                    value={local.sandbox.memoryLimitMb}
                    onChange={(e) =>
                      setLocal((p) => ({
                        ...p,
                        sandbox: { ...p.sandbox, memoryLimitMb: Number(e.target.value) }
                      }))
                    }
                  />
                </div>
              </>
            )}
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
                      return (
                        <div>
                          <div className="field">
                            <label className="field-label">{tr.name}</label>
                            <div className="config-item-header-row">
                              <input
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
                          </div>
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
                              {(vibeConfig.models || []).map((model) => (
                                <option key={model.name} value={model.alias || model.name}>
                                  {model.alias || model.name} ({model.provider})
                                </option>
                              ))}
                            </select>
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
                              {vibePrompts.map((prompt) => (
                                <option key={prompt.name} value={prompt.name}>
                                  {prompt.name}
                                </option>
                              ))}
                            </select>
                          </div>
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
                    <p className="empty-text">{tr.noToolsAvailable || 'No tools configured'}</p>
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
                          </div>
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.transport || 'Transport'}</label>
                          <select
                            className="field-select"
                            value={server.transport}
                            onChange={(e) => handleUpdateMcpServer(idx, { transport: e.target.value as 'http' | 'streamable-http' | 'stdio' })}
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
                              <label className="field-label">Headers (JSON)</label>
                              <input
                                className="field-input"
                                type="text"
                                value={JSON.stringify(server.headers || {})}
                                onChange={(e) => {
                                  try {
                                    handleUpdateMcpServer(idx, { headers: JSON.parse(e.target.value) })
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
                                value={server.api_key_env || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { api_key_env: e.target.value })}
                                placeholder="MCP_API_KEY"
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">API Key Header</label>
                              <input
                                className="field-input"
                                type="text"
                                value={server.api_key_header || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { api_key_header: e.target.value })}
                                placeholder="Authorization"
                              />
                            </div>
                            <div className="field">
                              <label className="field-label">API Key Format</label>
                              <input
                                className="field-input"
                                type="text"
                                value={server.api_key_format || ''}
                                onChange={(e) => handleUpdateMcpServer(idx, { api_key_format: e.target.value })}
                                placeholder="Bearer {token}"
                              />
                            </div>
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
                              <label className="field-label">Env (JSON)</label>
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
                          </>
                        )}
                        <div className="field">
                          <label className="field-label">Startup Timeout (sec)</label>
                          <input
                            className="field-input"
                            type="number"
                            value={server.startup_timeout_sec || 10}
                            onChange={(e) => handleUpdateMcpServer(idx, { startup_timeout_sec: Number(e.target.value) })}
                          />
                        </div>
                        <div className="field">
                          <label className="field-label">Tool Timeout (sec)</label>
                          <input
                            className="field-input"
                            type="number"
                            value={server.tool_timeout_sec || 60}
                            onChange={(e) => handleUpdateMcpServer(idx, { tool_timeout_sec: Number(e.target.value) })}
                          />
                        </div>
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
                  <p className="empty-text">{tr.noToolsAvailable || 'No tools configured'}</p>
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
              <button className="field-btn" onClick={handleOpenSkillsFolder}>
                {tr.openSkillsFolder}
              </button>
            </div>
            {!loaded ? (
              <p className="empty-text">{tr.loading}</p>
            ) : vibeSkills.length === 0 ? (
              <p className="empty-text">{tr.noVibeSkillsConfigured}</p>
            ) : (
              <div className="config-list">
                {vibeSkills.map((skill) => (
                  <div key={skill.name} className="config-item">
                    <div className="config-item-header">
                      <span className="config-item-name">{skill.name}</span>
                    </div>
                    <div className="field">
                      <textarea
                        className="field-textarea"
                        value={skill.content}
                        readOnly
                        rows={8}
                      />
                    </div>
                  </div>
                ))}
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
                        </div>
                        <div className="field">
                          <label className="field-label">{tr.thinking || 'Thinking'}</label>
                          <select
                            className="field-select"
                            value={model.thinking || 'off'}
                            onChange={(e) => handleUpdateModel(idx, { thinking: e.target.value as 'off' | 'low' | 'medium' | 'high' })}
                          >
                            <option value="off">{tr.off || 'Off'}</option>
                            <option value="low">{tr.low || 'Low'}</option>
                            <option value="medium">{tr.medium || 'Medium'}</option>
                            <option value="high">{tr.high || 'High'}</option>
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
                          <label className="field-label">{tr.autoCompactThreshold || 'Auto Compact Threshold'}</label>
                          <input
                            className="field-input"
                            type="number"
                            value={model.auto_compact_threshold ?? 200000}
                            onChange={(e) => handleUpdateModel(idx, { auto_compact_threshold: Number(e.target.value) })}
                            placeholder="200000"
                          />
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
      </div>

      <div className="settings-footer">
        <button className="save-btn" onClick={handleSave}>
          {saved ? tr.saved : tr.saveSettings}
        </button>
      </div>
    </div>
  )
}
