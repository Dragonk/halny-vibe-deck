import { contextBridge, ipcRenderer } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'

// Minimal type definitions (mirrored from main process)
interface AcpEvent {
  type: string
  data: Record<string, unknown>
}

interface AppConfig {
  version: string
  language: string
  theme: 'auto' | 'dark' | 'light'
  defaultModel: string
  sandboxEnabled: boolean
  sandbox: {
    enabled: boolean
    network: 'none' | 'whitelist' | 'unrestricted'
    whitelistDomains: string[]
    timeoutSeconds: number
    memoryLimitMb: number
  }
  acpPath: string
  systemPrompt: string
  workingDirectory: string
  recentFolders: string[]
  lastSessionId: string | null
}

interface SetupStatus {
  vibAcpFound: boolean
  vibAcpPath: string | null
  uvFound: boolean
  uvPath: string | null
}

interface SetupProgressEvent {
  message: string
  type: 'info' | 'error' | 'success'
}

interface VibeSession {
  sessionId: string
  startTime: string
  endTime: string
  title: string | null
  totalMessages: number
  gitBranch: string | null
  environment: { workingDirectory: string }
  stats: {
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
  config: { activeModel: string } | null
}

interface VibeToolCall {
  id: string
  index: number
  function: { name: string; arguments: string }
  type: 'function'
}

interface VibeSessionMessage {
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

interface VibeAgentConfig {
  name: string
  active_model?: string
  system_prompt_id?: string
  disabled_tools?: string[]
  enabled_tools?: string[]
}

interface VibeMcpServerConfig {
  name: string
  transport: 'http' | 'streamable-http' | 'stdio'
  url?: string
  headers?: Record<string, string>
  api_key_env?: string
  command?: string
  args?: string[]
  env?: Record<string, string>
}

interface VibeProviderConfig {
  name: string
  api_key_env?: string
  api_key?: string
  base_url?: string
}

interface VibeModelConfig {
  name: string
  provider: string
  enabled?: boolean
}

interface VibeConfig {
  agents?: VibeAgentConfig[]
  mcp_servers?: VibeMcpServerConfig[]
  tools?: {
    enabled_tools?: string[]
    disabled_tools?: string[]
  }
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

const api = {
  window: {
    minimize: (): Promise<void> => ipcRenderer.invoke('window:minimize'),
    maximize: (): Promise<void> => ipcRenderer.invoke('window:maximize'),
    close: (): Promise<void> => ipcRenderer.invoke('window:close')
  },
  config: {
    get: (): Promise<AppConfig> => ipcRenderer.invoke('config:get'),
    save: (config: AppConfig): Promise<void> => ipcRenderer.invoke('config:save', config)
  },
  dialog: {
    openFolder: (): Promise<string | null> => ipcRenderer.invoke('dialog:open-folder'),
    openFile: (): Promise<string | null> => ipcRenderer.invoke('dialog:open-file')
  },
  acp: {
    init: (acpPath: string): Promise<void> =>
      ipcRenderer.invoke('acp:init', acpPath),
    start: (acpPath: string, cwd: string, model: string): Promise<void> =>
      ipcRenderer.invoke('acp:start', acpPath, cwd, model),
    newSession: (cwd: string, model: string): Promise<void> =>
      ipcRenderer.invoke('acp:new-session', cwd, model),
    resumeSession: (sessionId: string, cwd: string, model: string): Promise<void> =>
      ipcRenderer.invoke('acp:resume-session', sessionId, cwd, model),
    loadSession: (sessionId: string, cwd: string, model: string): Promise<void> =>
      ipcRenderer.invoke('acp:load-session', sessionId, cwd, model),
    sendPrompt: (text: string, acpSessionId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:send-prompt', text, acpSessionId),
    cancelSession: (acpSessionId: string): Promise<void> =>
      ipcRenderer.invoke('acp:cancel-session', acpSessionId),
    setMode: (mode: string, sessionId?: string): Promise<void> => ipcRenderer.invoke('acp:set-mode', mode, sessionId),
    setModel: (model: string, sessionId?: string): Promise<void> => ipcRenderer.invoke('acp:set-model', model, sessionId),
    approveTool: (requestId: string, optionId: string): Promise<void> => ipcRenderer.invoke('acp:approve-tool', requestId, optionId),
    answerQuestion: (requestId: string, answer: string): Promise<void> => ipcRenderer.invoke('acp:answer-question', requestId, answer),
    stop: (): Promise<void> => ipcRenderer.invoke('acp:stop'),
    isRunning: (): Promise<boolean> => ipcRenderer.invoke('acp:is-running'),
    getContextSummary: (messages: Array<{ role: string; content: string }>): Promise<string> =>
      ipcRenderer.invoke('acp:get-context-summary', messages),
    onEvent: (cb: (event: AcpEvent) => void): (() => void) => {
      const listener = (_: Electron.IpcRendererEvent, event: AcpEvent): void => cb(event)
      ipcRenderer.on('acp:event', listener)
      return () => ipcRenderer.removeListener('acp:event', listener)
    }
  },
  stats: {
    vibeSessions: (): Promise<VibeSession[]> => ipcRenderer.invoke('stats:vibe-sessions'),
    vibeSessionMessages: (sessionId: string): Promise<VibeSessionMessage[]> =>
      ipcRenderer.invoke('stats:vibe-session-messages', sessionId),
    vibeSessionMetadata: (sessionId: string): Promise<{ activeModel: string | null; agentProfile: string | null }> =>
      ipcRenderer.invoke('stats:vibe-session-metadata', sessionId),
    vibeSubagentSessions: (parentSessionId: string): Promise<Array<{ sessionId: string; agentName: string; title: string | null; startTime: string; messages: VibeSessionMessage[] }>> =>
      ipcRenderer.invoke('stats:vibe-subagent-sessions', parentSessionId)
  },
  setup: {
    check: (): Promise<SetupStatus> => ipcRenderer.invoke('setup:check'),
    install: (): Promise<SetupStatus> => ipcRenderer.invoke('setup:install'),
    onProgress: (cb: (event: SetupProgressEvent) => void): (() => void) => {
      const listener = (_: Electron.IpcRendererEvent, event: SetupProgressEvent): void => cb(event)
      ipcRenderer.on('setup:progress', listener)
      return () => ipcRenderer.removeListener('setup:progress', listener)
    }
  },
  fs: {
    listDir: (dir: string): Promise<Array<{ name: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:list-dir', dir),
    searchFiles: (rootDir: string, query: string): Promise<Array<{ path: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:search-files', rootDir, query),
    readFile: (filePath: string): Promise<{ type: 'file' | 'directory' | 'error'; content: string }> =>
      ipcRenderer.invoke('fs:read-file', filePath)
  },
  shell: {
    openPath: (path: string): Promise<string> => ipcRenderer.invoke('shell:open-path', path)
  },
  vibe: {
    getConfig: (): Promise<VibeConfig> => ipcRenderer.invoke('vibe:get-config'),
    saveConfig: (config: VibeConfig): Promise<void> => ipcRenderer.invoke('vibe:save-config', config),
    getAgents: (): Promise<VibeAgentConfig[]> => ipcRenderer.invoke('vibe:get-agents'),
    saveAgent: (name: string, config: VibeAgentConfig): Promise<void> => ipcRenderer.invoke('vibe:save-agent', name, config),
    deleteAgent: (name: string): Promise<void> => ipcRenderer.invoke('vibe:delete-agent', name),
    getPrompts: (): Promise<VibePrompt[]> => ipcRenderer.invoke('vibe:get-prompts'),
    savePrompt: (name: string, content: string): Promise<void> => ipcRenderer.invoke('vibe:save-prompt', name, content),
    deletePrompt: (name: string): Promise<void> => ipcRenderer.invoke('vibe:delete-prompt', name),
    getSkills: (): Promise<VibeSkill[]> => ipcRenderer.invoke('vibe:get-skills'),
    getDir: (): Promise<string> => ipcRenderer.invoke('vibe:get-dir')
  }
}

if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld('electron', electronAPI)
    contextBridge.exposeInMainWorld('api', api)
  } catch (e) {
    console.error(e)
  }
} else {
  // @ts-ignore
  window.electron = electronAPI
  // @ts-ignore
  window.api = api
}

export type AppAPI = typeof api
