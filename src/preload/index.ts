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

/** Config scope for vibe settings: global (~/.vibe) or project (<cwd>/.vibe). */
export type VibeScope = 'global' | 'project'

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
    vibeSessionLastModel: (sessionId: string): Promise<string | null> =>
      ipcRenderer.invoke('stats:vibe-session-last-model', sessionId),
    searchSessionContent: (query: string): Promise<Array<{ sessionId: string; snippet: string }>> =>
      ipcRenderer.invoke('stats:search-session-content', query),
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
      ipcRenderer.invoke('fs:read-binary', filePath),
    writeText: (filePath: string, content: string): Promise<{ error: string | null }> =>
      ipcRenderer.invoke('fs:write-text', filePath, content)
  },
  git: {
    status: (cwd: string): Promise<GitStatus> => ipcRenderer.invoke('git:status', cwd),
    log: (cwd: string, max: number): Promise<GitCommit[]> => ipcRenderer.invoke('git:log', cwd, max),
    diffFile: (cwd: string, filePath: string): Promise<string | null> =>
      ipcRenderer.invoke('git:diff-file', cwd, filePath),
    diffAll: (cwd: string): Promise<string | null> => ipcRenderer.invoke('git:diff-all', cwd),
    stage: (cwd: string, paths: string[]): Promise<string | null> =>
      ipcRenderer.invoke('git:stage', cwd, paths),
    unstage: (cwd: string, paths: string[]): Promise<string | null> =>
      ipcRenderer.invoke('git:unstage', cwd, paths),
    commit: (cwd: string, message: string): Promise<string | null> =>
      ipcRenderer.invoke('git:commit', cwd, message),
    push: (cwd: string): Promise<string | null> =>
      ipcRenderer.invoke('git:push', cwd)
  },
  shell: {
    openPath: (path: string): Promise<string> => ipcRenderer.invoke('shell:open-path', path)
  },
  vibe: {
    // scope/cwd select the global (~/.vibe) or the project (<cwd>/.vibe) config;
    // they default to the global scope so legacy no-argument calls keep working
    getConfig: (scope?: VibeScope, cwd?: string): Promise<unknown> =>
      ipcRenderer.invoke('vibe:get-config', scope ?? 'global', cwd ?? ''),
    saveConfig: (config: unknown, scope?: VibeScope, cwd?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-config', config, scope ?? 'global', cwd ?? ''),
    getRawConfig: (scope?: VibeScope, cwd?: string): Promise<{ content: string; error: string | null }> =>
      ipcRenderer.invoke('vibe:get-raw-config', scope ?? 'global', cwd ?? ''),
    saveRawConfig: (content: string, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:save-raw-config', content, scope ?? 'global', cwd ?? ''),
    getAgents: (scope?: VibeScope, cwd?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-agents', scope ?? 'global', cwd ?? ''),
    saveAgent: (name: string, config: unknown, scope?: VibeScope, cwd?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-agent', name, config, scope ?? 'global', cwd ?? ''),
    deleteAgent: (name: string, scope?: VibeScope, cwd?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:delete-agent', name, scope ?? 'global', cwd ?? ''),
    getPrompts: (scope?: VibeScope, cwd?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-prompts', scope ?? 'global', cwd ?? ''),
    savePrompt: (name: string, content: string, scope?: VibeScope, cwd?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-prompt', name, content, scope ?? 'global', cwd ?? ''),
    deletePrompt: (name: string, scope?: VibeScope, cwd?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:delete-prompt', name, scope ?? 'global', cwd ?? ''),
    getSkills: (scope?: VibeScope, cwd?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-skills', scope ?? 'global', cwd ?? ''),
    getDir: (scope?: VibeScope, cwd?: string): Promise<string> =>
      ipcRenderer.invoke('vibe:get-dir', scope ?? 'global', cwd ?? ''),
    listConfigFiles: (scope?: VibeScope, cwd?: string): Promise<Array<{ path: string; full: string }>> =>
      ipcRenderer.invoke('vibe:list-config-files', scope ?? 'global', cwd ?? ''),
    createSkill: (name: string, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:create-skill', name, scope ?? 'global', cwd ?? ''),
    deleteSkill: (name: string, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:delete-skill', name, scope ?? 'global', cwd ?? ''),
    writeSkillFile: (name: string, relPath: string, content: string, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:write-skill-file', name, relPath, content, scope ?? 'global', cwd ?? ''),
    deleteSkillFile: (name: string, relPath: string, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:delete-skill-file', name, relPath, scope ?? 'global', cwd ?? ''),
    importSkill: (src: string, isZip: boolean, scope?: VibeScope, cwd?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:import-skill', src, isZip, scope ?? 'global', cwd ?? '')
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
