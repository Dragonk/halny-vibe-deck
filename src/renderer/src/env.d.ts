export interface VibeSessionMetadata {
  activeModel: string | null
  agentProfile: string | null
}

declare global {
  interface Window {
    api: {
      window: {
        minimize: () => Promise<void>
        maximize: () => Promise<void>
        close: () => Promise<void>
      }
      config: {
        get: () => Promise<unknown>
        save: (config: unknown) => Promise<void>
      }
      dialog: {
        openFolder: () => Promise<string | null>
        openFile: () => Promise<string | null>
      }
      acp: {
        init: (acpPath: string) => Promise<void>
        start: (acpPath: string, cwd: string, model: string) => Promise<void>
        newSession: (cwd: string, model: string) => Promise<void>
        resumeSession: (sessionId: string, cwd: string, model: string) => Promise<void>
        loadSession: (sessionId: string, cwd: string, model: string) => Promise<void>
        sendPrompt: (text: string, acpSessionId?: string) => Promise<void>
        cancelSession: (acpSessionId: string) => Promise<void>
        setMode: (mode: string, sessionId?: string) => Promise<void>
        setModel: (model: string, sessionId?: string) => Promise<void>
        approveTool: (requestId: string, optionId: string) => Promise<void>
        stop: () => Promise<void>
        isRunning: () => Promise<boolean>
        getContextSummary: (messages: Array<{ role: string; content: string }>) => Promise<string>
        onEvent: (cb: (event: unknown) => void) => () => void
      }
      stats: {
        vibeSessions: () => Promise<unknown[]>
        vibeSessionMessages: (sessionId: string) => Promise<unknown[]>
        vibeSessionMetadata: (sessionId: string) => Promise<VibeSessionMetadata>
      }
      setup: {
        check: () => Promise<unknown>
        install: () => Promise<unknown>
        onProgress: (cb: (event: unknown) => void) => () => void
      }
      fs: {
        listDir: (dir: string) => Promise<Array<{ name: string; isDir: boolean }>>
        searchFiles: (rootDir: string, query: string) => Promise<Array<{ path: string; isDir: boolean }>>
        readFile: (filePath: string) => Promise<{ type: 'file' | 'directory' | 'error'; content: string }>
      }
      shell: {
        openPath: (path: string) => Promise<string>
      }
      vibe: {
        getConfig: () => Promise<unknown>
        saveConfig: (config: unknown) => Promise<void>
        getAgents: () => Promise<unknown[]>
        saveAgent: (name: string, config: unknown) => Promise<void>
        deleteAgent: (name: string) => Promise<void>
        getPrompts: () => Promise<unknown[]>
        savePrompt: (name: string, config: unknown) => Promise<void>
        deletePrompt: (name: string) => Promise<void>
        getSkills: () => Promise<unknown[]>
        saveSkill: (name: string, config: unknown) => Promise<void>
        deleteSkill: (name: string) => Promise<void>
      }
    }
  }
}

export {}
