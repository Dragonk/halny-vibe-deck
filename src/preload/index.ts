import { contextBridge, ipcRenderer } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'

export interface AcpConfigOptionChoice {
  value: string
  name: string
  description?: string
}

export interface AcpConfigOption {
  id: string
  name: string
  description?: string
  category?: string
  type: 'select' | 'boolean'
  currentValue: string | boolean
  options: AcpConfigOptionChoice[]
}

export interface AcpEvent {
  type: string
  data: Record<string, unknown>
}

export interface GitFileStatus {
  path: string
  index: string
  working: string
  staged: boolean
}

export interface GitStatus {
  repo: boolean
  branch: string | null
  ahead: number
  behind: number
  files: GitFileStatus[]
  error?: string
}

export interface GitCommit {
  hash: string
  shortHash: string
  message: string
  author: string
  date: string
}

const api = {
  window: {
    minimize: (): Promise<void> => ipcRenderer.invoke('window:minimize'),
    maximize: (): Promise<void> => ipcRenderer.invoke('window:maximize'),
    close: (): Promise<void> => ipcRenderer.invoke('window:close')
  },
  config: {
    get: (): Promise<unknown> => ipcRenderer.invoke('config:get'),
    save: (config: unknown): Promise<void> => ipcRenderer.invoke('config:save', config)
  },
  dialog: {
    openFolder: (): Promise<string | null> => ipcRenderer.invoke('dialog:open-folder'),
    openFile: (): Promise<string | null> => ipcRenderer.invoke('dialog:open-file')
  },
  acp: {
    init: (acpPath: string): Promise<void> => ipcRenderer.invoke('acp:init', acpPath),
    start: (acpPath: string, cwd: string): Promise<void> => ipcRenderer.invoke('acp:start', acpPath, cwd),
    newSession: (cwd: string): Promise<void> => ipcRenderer.invoke('acp:new-session', cwd),
    loadSession: (sessionId: string, cwd: string): Promise<void> =>
      ipcRenderer.invoke('acp:load-session', sessionId, cwd),
    sendPrompt: (text: string, acpSessionId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:send-prompt', text, acpSessionId),
    cancelSession: (acpSessionId: string): Promise<void> =>
      ipcRenderer.invoke('acp:cancel-session', acpSessionId),
    setConfigOption: (key: string, value: string, sessionId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:set-config-option', key, value, sessionId),
    approveTool: (requestId: string, optionId: string): Promise<void> =>
      ipcRenderer.invoke('acp:approve-tool', requestId, optionId),
    answerQuestion: (requestId: string, answer: string): Promise<void> =>
      ipcRenderer.invoke('acp:answer-question', requestId, answer),
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
    vibeSessions: (): Promise<unknown[]> => ipcRenderer.invoke('stats:vibe-sessions'),
    vibeSessionMessages: (sessionId: string): Promise<unknown[]> =>
      ipcRenderer.invoke('stats:vibe-session-messages', sessionId),
    vibeSessionMetadata: (
      sessionId: string
    ): Promise<{ activeModel: string | null; agentProfile: string | null }> =>
      ipcRenderer.invoke('stats:vibe-session-metadata', sessionId),
    vibeSubagentSessions: (parentSessionId: string): Promise<unknown[]> =>
      ipcRenderer.invoke('stats:vibe-subagent-sessions', parentSessionId)
  },
  setup: {
    check: (): Promise<unknown> => ipcRenderer.invoke('setup:check'),
    install: (): Promise<unknown> => ipcRenderer.invoke('setup:install'),
    onProgress: (cb: (event: { message: string; type: string }) => void): (() => void) => {
      const listener = (_: Electron.IpcRendererEvent, event: { message: string; type: string }): void => cb(event)
      ipcRenderer.on('setup:progress', listener)
      return () => ipcRenderer.removeListener('setup:progress', listener)
    }
  },
  fs: {
    listDir: (dir: string): Promise<Array<{ name: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:list-dir', dir),
    searchFiles: (rootDir: string, query: string): Promise<Array<{ path: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:search-files', rootDir, query),
    readFile: (
      filePath: string
    ): Promise<{ type: 'file' | 'directory' | 'error'; content: string }> =>
      ipcRenderer.invoke('fs:read-file', filePath),
    stat: (filePath: string): Promise<{ exists: boolean; isDir: boolean; size: number; ext: string }> =>
      ipcRenderer.invoke('fs:stat', filePath),
    readText: (
      filePath: string
    ): Promise<{ content: string; truncated: boolean; size: number; error: string | null }> =>
      ipcRenderer.invoke('fs:read-text', filePath),
    readBinary: (
      filePath: string
    ): Promise<{ base64: string; mime: string; size: number; error: string | null }> =>
      ipcRenderer.invoke('fs:read-binary', filePath)
  },
  git: {
    status: (cwd: string): Promise<GitStatus> => ipcRenderer.invoke('git:status', cwd),
    log: (cwd: string, max: number): Promise<GitCommit[]> => ipcRenderer.invoke('git:log', cwd, max),
    diffFile: (cwd: string, filePath: string): Promise<string | null> =>
      ipcRenderer.invoke('git:diff-file', cwd, filePath),
    diffAll: (cwd: string): Promise<string | null> => ipcRenderer.invoke('git:diff-all', cwd)
  },
  shell: {
    openPath: (path: string): Promise<string> => ipcRenderer.invoke('shell:open-path', path)
  },
  vibe: {
    getConfig: (): Promise<unknown> => ipcRenderer.invoke('vibe:get-config'),
    saveConfig: (config: unknown): Promise<void> => ipcRenderer.invoke('vibe:save-config', config),
    getRawConfig: (): Promise<{ content: string; error: string | null }> =>
      ipcRenderer.invoke('vibe:get-raw-config'),
    saveRawConfig: (content: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:save-raw-config', content),
    getAgents: (): Promise<unknown[]> => ipcRenderer.invoke('vibe:get-agents'),
    saveAgent: (name: string, config: unknown): Promise<void> =>
      ipcRenderer.invoke('vibe:save-agent', name, config),
    deleteAgent: (name: string): Promise<void> => ipcRenderer.invoke('vibe:delete-agent', name),
    getPrompts: (): Promise<unknown[]> => ipcRenderer.invoke('vibe:get-prompts'),
    savePrompt: (name: string, content: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-prompt', name, content),
    deletePrompt: (name: string): Promise<void> => ipcRenderer.invoke('vibe:delete-prompt', name),
    getSkills: (): Promise<unknown[]> => ipcRenderer.invoke('vibe:get-skills'),
    getDir: (): Promise<string> => ipcRenderer.invoke('vibe:get-dir')
  }
}

export type AppAPI = typeof api

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
