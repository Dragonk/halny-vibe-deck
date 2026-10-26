import { contextBridge, ipcRenderer, webUtils } from 'electron'
import { electronAPI } from '@electron-toolkit/preload'

export interface AcpConfigOptionChoice {
  value: string
  name: string
  description?: string
}

export interface AcpApplyConfig {
  model?: string
  mode?: string
  thinking?: string
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
export type VibeScope = 'global' | 'project' | 'remote'

/** Serve (HTTPS web hosting) status */
export interface ServeStatus {
  running: boolean
  port: number
  url: string
  lanUrls: string[]
}

/** Serve settings editable from the local UI (credentials excluded) */
export interface ServePatch {
  enabled?: boolean
  port?: number
  certPath?: string
  keyPath?: string
}

/** SSH remote connection (draft or saved) */
export interface RemoteConnection {
  id: string
  name: string
  enabled: boolean
  sshCmd: string
  locations: string[]
  installCommand: string
}

export interface RemoteTestResult {
  reachable: boolean
  acpInstalled: boolean
  acpPath: string | null
  version: string | null
  error: string | null
}

export interface RemoteSessionInfo {
  sessionId: string
  title: string
  cwd: string
  updatedAt: string
}

export interface RemoteStatus {
  id: string
  ok: boolean
  checkedAt: number
  latencyMs: number | null
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
    start: (acpPath: string, cwd: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:start', acpPath, cwd, remoteId ?? undefined),
    ensureWorker: (acpPath: string, cwd: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:ensure-worker', acpPath, cwd, remoteId ?? undefined),
    stopWorker: (remoteId?: string): Promise<void> => ipcRenderer.invoke('acp:stop-worker', remoteId ?? undefined),
    newSession: (cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig): Promise<void> =>
      ipcRenderer.invoke('acp:new-session', cwd, remoteId ?? undefined, applyConfig ?? undefined),
    loadSession: (sessionId: string, cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig): Promise<void> =>
      ipcRenderer.invoke('acp:load-session', sessionId, cwd, remoteId ?? undefined, applyConfig ?? undefined),
    sendPrompt: (text: string, acpSessionId?: string, remoteId?: string, images?: Array<{ data: string; mimeType: string; uri?: string }>): Promise<void> =>
      ipcRenderer.invoke('acp:send-prompt', text, acpSessionId, remoteId ?? undefined, images ?? []),
    cancelSession: (acpSessionId: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:cancel-session', acpSessionId, remoteId ?? undefined),
    setConfigOption: (key: string, value: string, sessionId?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:set-config-option', key, value, sessionId, remoteId ?? undefined),
    approveTool: (requestId: string, optionId: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:approve-tool', requestId, optionId, remoteId ?? undefined),
    answerQuestion: (requestId: string, answer: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('acp:answer-question', requestId, answer, remoteId ?? undefined),
    stop: (): Promise<void> => ipcRenderer.invoke('acp:stop'),
    isRunning: (remoteId?: string): Promise<boolean> => ipcRenderer.invoke('acp:is-running', remoteId ?? undefined),
    getContextSummary: (messages: Array<{ role: string; content: string }>, remoteId?: string): Promise<string> =>
      ipcRenderer.invoke('acp:get-context-summary', messages, remoteId ?? undefined),
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
    vibeSessionLastConfig: (
      sessionId: string
    ): Promise<{ model: string | null; mode: string | null; thinking: string | null; cwd: string | null } | null> =>
      ipcRenderer.invoke('stats:vibe-session-last-config', sessionId),
    searchSessionContent: (query: string): Promise<Array<{ sessionId: string; snippet: string }>> =>
      ipcRenderer.invoke('stats:search-session-content', query),
    vibeSessionMessages: (sessionId: string): Promise<unknown[]> =>
      ipcRenderer.invoke('stats:vibe-session-messages', sessionId),
    sessionPreview: (sessionId: string): Promise<unknown> =>
      ipcRenderer.invoke('stats:session-preview', sessionId),
    remoteSessionPreview: (remoteId: string, sessionId: string): Promise<unknown> =>
      ipcRenderer.invoke('stats:remote-session-preview', remoteId, sessionId),
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
    listDir: (dir: string, remoteId?: string): Promise<Array<{ name: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:list-dir', dir, remoteId ?? undefined),
    homeDir: (remoteId?: string): Promise<string> =>
      ipcRenderer.invoke('fs:home-dir', remoteId ?? undefined),
    searchFiles: (rootDir: string, query: string, remoteId?: string): Promise<Array<{ path: string; isDir: boolean }>> =>
      ipcRenderer.invoke('fs:search-files', rootDir, query, remoteId ?? undefined),
    readFile: (
      filePath: string,
      remoteId?: string
    ): Promise<{ type: 'file' | 'directory' | 'error'; content: string }> =>
      ipcRenderer.invoke('fs:read-file', filePath, remoteId ?? undefined),
    stat: (filePath: string, remoteId?: string): Promise<{ exists: boolean; isDir: boolean; size: number; ext: string }> =>
      ipcRenderer.invoke('fs:stat', filePath, remoteId ?? undefined),
    readText: (
      filePath: string,
      remoteId?: string
    ): Promise<{ content: string; truncated: boolean; size: number; error: string | null }> =>
      ipcRenderer.invoke('fs:read-text', filePath, remoteId ?? undefined),
    readBinary: (
      filePath: string,
      remoteId?: string
    ): Promise<{ base64: string; mime: string; size: number; error: string | null }> =>
      ipcRenderer.invoke('fs:read-binary', filePath, remoteId ?? undefined),
    writeText: (filePath: string, content: string, remoteId?: string): Promise<{ error: string | null }> =>
      ipcRenderer.invoke('fs:write-text', filePath, content, remoteId ?? undefined),
    writeBinary: (filePath: string, base64: string, remoteId?: string): Promise<{ error: string | null }> =>
      ipcRenderer.invoke('fs:write-binary', filePath, base64, remoteId ?? undefined),
    // v1.0.8: absolute host path of a dropped File (Electron webUtils) -
    // null in the browser (the drop handler uploads the bytes instead).
    pathForFile: (file: File): string | null => {
      try {
        return webUtils.getPathForFile(file)
      } catch {
        return null
      }
    },
    mkdir: (filePath: string, remoteId?: string): Promise<{ error: string | null }> =>
      ipcRenderer.invoke('fs:mkdir', filePath, remoteId ?? undefined)
  },
  git: {
    status: (cwd: string, remoteId?: string): Promise<GitStatus> =>
      ipcRenderer.invoke('git:status', cwd, remoteId ?? undefined),
    log: (cwd: string, max: number, remoteId?: string): Promise<GitCommit[]> =>
      ipcRenderer.invoke('git:log', cwd, max, remoteId ?? undefined),
    diffFile: (cwd: string, filePath: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:diff-file', cwd, filePath, remoteId ?? undefined),
    diffAll: (cwd: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:diff-all', cwd, remoteId ?? undefined),
    stage: (cwd: string, paths: string[], remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:stage', cwd, paths, remoteId ?? undefined),
    unstage: (cwd: string, paths: string[], remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:unstage', cwd, paths, remoteId ?? undefined),
    commit: (cwd: string, message: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:commit', cwd, message, remoteId ?? undefined),
    push: (cwd: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('git:push', cwd, remoteId ?? undefined)
  },

  shell: {
    openPath: (path: string): Promise<string> => ipcRenderer.invoke('shell:open-path', path)
  },
  vibe: {
    // scope/cwd select the global (~/.vibe) or the project (<cwd>/.vibe) config;
    // they default to the global scope so legacy no-argument calls keep working
    getConfig: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<unknown> =>
      ipcRenderer.invoke('vibe:get-config', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    saveConfig: (config: unknown, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-config', config, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    getRawConfig: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<{ content: string; error: string | null }> =>
      ipcRenderer.invoke('vibe:get-raw-config', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    saveRawConfig: (content: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:save-raw-config', content, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    getAgents: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-agents', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    exposeProjectAgent: (name: string, cwd: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:expose-project-agent', name, cwd, remoteId ?? ''),
    saveAgent: (name: string, config: unknown, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-agent', name, config, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    deleteAgent: (name: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:delete-agent', name, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    getPrompts: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-prompts', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    savePrompt: (name: string, content: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:save-prompt', name, content, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    deletePrompt: (name: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<void> =>
      ipcRenderer.invoke('vibe:delete-prompt', name, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    getSkills: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<unknown[]> =>
      ipcRenderer.invoke('vibe:get-skills', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    getDir: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string> =>
      ipcRenderer.invoke('vibe:get-dir', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    listConfigFiles: (scope?: VibeScope, cwd?: string, remoteId?: string): Promise<Array<{ path: string; full: string }>> =>
      ipcRenderer.invoke('vibe:list-config-files', scope ?? 'global', cwd ?? '', remoteId ?? ''),
    createSkill: (name: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:create-skill', name, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    deleteSkill: (name: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:delete-skill', name, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    writeSkillFile: (name: string, relPath: string, content: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:write-skill-file', name, relPath, content, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    deleteSkillFile: (name: string, relPath: string, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:delete-skill-file', name, relPath, scope ?? 'global', cwd ?? '', remoteId ?? ''),
    importSkill: (src: string, isZip: boolean, scope?: VibeScope, cwd?: string, remoteId?: string): Promise<string | null> =>
      ipcRenderer.invoke('vibe:import-skill', src, isZip, scope ?? 'global', cwd ?? '', remoteId ?? '')
  },
  serve: {
    status: (): Promise<ServeStatus> => ipcRenderer.invoke('serve:status'),
    start: (): Promise<ServeStatus> => ipcRenderer.invoke('serve:start'),
    stop: (): Promise<ServeStatus> => ipcRenderer.invoke('serve:stop'),
    saveConfig: (patch: ServePatch): Promise<ServeStatus> => ipcRenderer.invoke('serve:save-config', patch),
    setCredentials: (username: string, password: string): Promise<{ ok: boolean }> =>
      ipcRenderer.invoke('serve:set-credentials', username, password),
    regenCert: (): Promise<{ ok: boolean }> => ipcRenderer.invoke('serve:regen-cert')
  },
  updates: {
    checkApp: (): Promise<unknown> => ipcRenderer.invoke('updates:check-app'),
    checkCli: (): Promise<unknown> => ipcRenderer.invoke('updates:check-cli'),
    checkCliRemote: (remoteId: string): Promise<unknown> =>
      ipcRenderer.invoke('updates:check-cli-remote', remoteId),
    updateCli: (remoteId?: string): Promise<unknown> =>
      ipcRenderer.invoke('updates:update-cli', remoteId ?? null),
    updateApp: (): Promise<unknown> => ipcRenderer.invoke('updates:update-app'),
    onProgress: (listener: (p: unknown) => void): (() => void) => {
      const l = (_e: unknown, p: unknown): void => listener(p)
      ipcRenderer.on('updates:progress', l)
      return () => ipcRenderer.removeListener('updates:progress', l)
    }
  },
  remote: {
    test: (draft: RemoteConnection): Promise<RemoteTestResult> => ipcRenderer.invoke('remote:test', draft),
    installAcp: (id: string): Promise<{ ok: boolean; error: string | null; log: string }> =>
      ipcRenderer.invoke('remote:install-acp', id),
    sessions: (id: string): Promise<RemoteSessionInfo[]> => ipcRenderer.invoke('remote:sessions', id),
    statusNow: (ids?: string[]): Promise<Record<string, RemoteStatus>> =>
      ipcRenderer.invoke('remote:status-now', ids ?? null),
    save: (conn: RemoteConnection): Promise<RemoteConnection> => ipcRenderer.invoke('remote:save', conn),
    delete: (id: string): Promise<{ ok: boolean }> => ipcRenderer.invoke('remote:delete', id)
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
