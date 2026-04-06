import React, { useEffect, useRef, useCallback, useState } from 'react'
import { useStore } from './store'
import { useTranslation, t } from './i18n'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import SettingsView from './components/SettingsView'
import StatsView from './components/StatsView'
import SetupView from './components/SetupView'

interface AcpEvent {
  type: string
  data: Record<string, unknown>
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

declare global {
  interface Window {
    api: {
      window: {
        minimize: () => Promise<void>
        maximize: () => Promise<void>
        close: () => Promise<void>
      }
      config: {
        get: () => Promise<import('./types').AppConfig>
        save: (config: import('./types').AppConfig) => Promise<void>
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
        answerQuestion: (requestId: string, answer: string) => Promise<void>
        stop: () => Promise<void>
        isRunning: () => Promise<boolean>
        getContextSummary: (messages: Array<{ role: string; content: string }>) => Promise<string>
        onEvent: (cb: (event: AcpEvent) => void) => () => void
      }
      stats: {
        vibeSessions: () => Promise<import('./types').VibeSession[]>
        vibeSessionMessages: (sessionId: string) => Promise<import('./types').VibeSessionMessage[]>
        vibeSubagentSessions: (parentSessionId: string) => Promise<Array<{ sessionId: string; agentName: string; title: string | null; startTime: string; messages: import('./types').VibeSessionMessage[] }>>
      }
      setup: {
        check: () => Promise<SetupStatus>
        install: () => Promise<SetupStatus>
        onProgress: (cb: (event: SetupProgressEvent) => void) => () => void
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
        getConfig: () => Promise<VibeConfig>
        saveConfig: (config: VibeConfig) => Promise<void>
        getAgents: () => Promise<VibeAgentConfig[]>
        saveAgent: (name: string, config: VibeAgentConfig) => Promise<void>
        deleteAgent: (name: string) => Promise<void>
        getPrompts: () => Promise<VibePrompt[]>
        savePrompt: (name: string, content: string) => Promise<void>
        deletePrompt: (name: string) => Promise<void>
        getSkills: () => Promise<VibeSkill[]>
        getDir: () => Promise<string>
      }
    }
  }
}

export default function App(): React.JSX.Element {
  const {
    theme,
    config,
    currentView,
    setConfig,
    setAcpStatus,
    setAcpError,
    setCurrentModel,
    setAvailableModels,
    setCurrentMode,
    setAvailableModes,
    setAvailableCommands,
    setWorkingDirectory,
    setSandboxEnabled,
    startAiMessage,
    appendChunk,
    finalizeAiMessage,
    updateToolCall,
    mergeToolCall,
    startThinking,
    appendThinkingChunk,
    finalizeThinking,
    linkAcpSession,
    getSessionByAcpId,
    importAcpSessions,
    setPendingNewSession,
    setSessionStarting,
    currentSessionId,
    messageQueue,
    getNextQueuedMessage,
    removeFromMessageQueue,
    addUserMessage,
    updateSession,
    sessions,
    markSessionBusy,
    markSessionIdle,
  } = useStore()

  const tr = useTranslation(config.language)

  // Per-session streaming state: localSessionId → { msgId, thinkingBlockId }
  const streamingState = useRef<Map<string, { msgId: string; thinkingBlockId: string | null }>>(new Map())
  const [setupRequired, setSetupRequired] = useState(false)
  const [pendingToolApprovals, setPendingToolApprovals] = useState<Array<{
    requestId: string
    method: string
    params: Record<string, unknown>
  }>>([])

  const [pendingQuestion, setPendingQuestion] = useState<{
    requestId: string
    question: string
    choices: string[]
  } | null>(null)

  const handleAcpEvent = useCallback(
    (event: AcpEvent) => {
      const d = event.data as Record<string, unknown>
      switch (event.type) {
        case 'InitializationComplete':
          setAcpStatus('connected')
          // Create a session immediately to get available models and modes
          // Use "." as current directory since we just need to initialize the session
          window.api.acp.newSession(".", "").catch(console.error)
          break

        case 'SessionsList': {
          const sessions = d.sessions as Array<{ sessionId: string; name: string; cwd: string; startTime: string }>
          importAcpSessions(sessions)
          break
        }

        case 'SessionInitialized': {
          const acpSessId = d.sessionId as string
          const modes = d.modes as Array<{ id: string; name: string }>
          const models = d.models as string[]
          const sessionTitle = d.sessionTitle as string | undefined
          console.log('[App] SessionInitialized:', acpSessId, 'local currentSessionId:', currentSessionId)
          setAvailableModes(modes)
          setAvailableModels(models)
          if (modes.length > 0) setCurrentMode(modes[0].id)
          if (models.length > 0) {
            const store = useStore.getState()
            if (!store.currentModel || !models.includes(store.currentModel)) {
              setCurrentModel(models[0])
            }
          }

          const store = useStore.getState()
          // First check if this ACP session was already imported (from session/list)
          const existing = store.getSessionByAcpId(acpSessId)
          console.log('[App] existing session found:', existing?.id, 'currentSessionId in store:', store.currentSessionId)
          if (existing) {
            // Session already in list — make it current, update name if available
            store.setCurrentSessionId(existing.id)
            if (sessionTitle) store.updateSession(existing.id, { name: sessionTitle })
          } else {
            // Brand-new session — link to current local session or create one
            const localSessId = store.currentSessionId
            console.log('[App] linking new ACP session to local:', localSessId)
            if (localSessId) {
              linkAcpSession(localSessId, acpSessId)
              if (sessionTitle) store.updateSession(localSessId, { name: sessionTitle })
            } else {
              const newSess = store.addSession()
              console.log('[App] created new local session:', newSess.id)
              linkAcpSession(newSess.id, acpSessId)
              if (sessionTitle) store.updateSession(newSess.id, { name: sessionTitle })
            }
          }
          setSessionStarting(false)
          break
        }

        case 'MessageChunk': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const localSess = store.getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          console.log('[App] MessageChunk:', acpSessId, '-> localId:', localId)
          if (localId) {
            markSessionBusy(localId)
            const state = streamingState.current.get(localId)
            if (!state) {
              console.log('[App] Starting new AI message for localId:', localId)
              const msgId = startAiMessage(localId)
              streamingState.current.set(localId, { msgId, thinkingBlockId: null })
              appendChunk(localId, msgId, d.text as string)
            } else {
              appendChunk(localId, state.msgId, d.text as string)
            }
          }
          break
        }

        case 'ThinkingChunk': {
          const acpSessId = d.sessionId as string
          const localSess = getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          const blockId = (d.blockId as string) ?? 'thinking'
          if (localId) {
            let state = streamingState.current.get(localId)
            if (!state) {
              const msgId = startAiMessage(localId)
              state = { msgId, thinkingBlockId: null }
              streamingState.current.set(localId, state)
            }
            if (state.thinkingBlockId !== blockId) {
              state.thinkingBlockId = blockId
              startThinking(localId, state.msgId, blockId)
            }
            appendThinkingChunk(localId, state.msgId, blockId, d.text as string)
          }
          break
        }

        case 'ThinkingComplete': {
          const acpSessId = d.sessionId as string
          const localSess = getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          const blockId = (d.blockId as string) ?? 'thinking'
          const state = localId ? streamingState.current.get(localId) : null
          if (localId && state) {
            finalizeThinking(localId, state.msgId, blockId)
            state.thinkingBlockId = null
          }
          break
        }

        case 'PromptComplete': {
          const acpSessId = d.sessionId as string
          const localSess = getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          const state = localId ? streamingState.current.get(localId) : null
          console.log('[App] PromptComplete:', acpSessId, '-> localId:', localId, 'msgId:', state?.msgId)
          if (localId && state) {
            finalizeAiMessage(localId, state.msgId)
            streamingState.current.delete(localId)
          }
          if (localId) markSessionIdle(localId)
          setAcpStatus('connected')

          // Process message queue - send next queued message
          const store = useStore.getState()
          const queue = store.messageQueue
          if (queue.length > 0 && localId) {
            const nextMsg = queue[0]
            removeFromMessageQueue(nextMsg.id)
            addUserMessage(nextMsg.content)
            const sess = store.sessions.find((s) => s.id === localId)
            const promptText = sess?.contextSummary
              ? `${sess.contextSummary}\n\nUser: ${nextMsg.content}`
              : nextMsg.content
            window.api.acp.sendPrompt(promptText, acpSessId).catch((err) => {
              setAcpError((err as Error).message)
            })
          }
          break
        }

        case 'ToolCall': {
          const acpSessId = d.sessionId as string
          const localSess = getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          if (localId) {
            markSessionBusy(localId)
            // Ensure there's an active AI message to attach tool calls to.
            // If no streaming state exists yet, create a new (empty) AI message.
            if (!streamingState.current.has(localId)) {
              const msgId = startAiMessage(localId)
              streamingState.current.set(localId, { msgId, thinkingBlockId: null })
            }
            updateToolCall(localId, {
              id: d.toolCallId as string,
              title: (d.title as string) ?? '',
              kind: (d.kind as string) ?? '',
              status: (d.status as 'running' | 'done' | 'error') ?? 'running',
              input: d.input as Record<string, unknown> | undefined,
              fileChanges: d.fileChanges as import('./types').FileChange[] | undefined,
            })
          }
          break
        }
        case 'ToolCallUpdate': {
          const acpSessId = d.sessionId as string
          const localSess = getSessionByAcpId(acpSessId)
          const localId = localSess?.id ?? currentSessionId
          if (localId) {
            // Merge — only update status and output, preserve title/kind/input
            mergeToolCall(localId, d.toolCallId as string, {
              status: (d.status as 'running' | 'done' | 'error') ?? 'running',
              output: d.output as string | undefined,
            })
          }
          break
        }

        case 'ToolApprovalRequest': {
          const requestId = d.requestId as string
          const method = d.method as string
          const params = d.params as Record<string, unknown>
          setPendingToolApprovals((prev) => [...prev, { requestId, method, params }])
          break
        }

        case 'AskUserQuestion': {
          const requestId = d.requestId as string
          const question = d.question as string
          const choices = d.choices as string[]
          setPendingQuestion({ requestId, question, choices })
          break
        }

        case 'Error':
          setAcpError(d.error as string)
          setAcpStatus('error')
          // Finalize any in-flight streaming messages
          streamingState.current.forEach(({ msgId }, localId) => {
            finalizeAiMessage(localId, msgId)
          })
          streamingState.current.clear()
          break

        case 'Disconnected':
          setAcpStatus('disconnected')
          break

        case 'DebugLog':
          break

        case 'AvailableCommandsUpdate': {
          const commands = d.commands as Array<{ name: string; description: string; hint?: string }>
          setAvailableCommands(commands)
          break
        }
      }
    },
    [
      setAcpStatus,
      setAcpError,
      setCurrentModel,
      setAvailableModels,
      setCurrentMode,
      setAvailableModes,
      setAvailableCommands,
      startAiMessage,
      appendChunk,
      finalizeAiMessage,
      updateToolCall,
      mergeToolCall,
      startThinking,
      appendThinkingChunk,
      finalizeThinking,
      linkAcpSession,
      getSessionByAcpId,
      importAcpSessions,
      setPendingNewSession,
      setSessionStarting,
      currentSessionId,
      streamingState,
      messageQueue,
      getNextQueuedMessage,
      removeFromMessageQueue,
      addUserMessage,
      sessions,
      setAcpError,
      markSessionBusy,
      markSessionIdle,
    ]
  )

  const startAcp = useCallback(
    (acpPath: string) => {
      setAcpStatus('connecting')
      window.api.acp
        .init(acpPath)
        .then(() => setPendingNewSession(true))
        .catch((err: Error) => {
          setAcpStatus('error')
          setAcpError(err.message)
        })
    },
    [setAcpStatus, setAcpError, setPendingNewSession]
  )

  // Load config and bootstrap
  useEffect(() => {
    Promise.all([window.api.config.get(), window.api.setup.check()]).then(([cfg, setupStatus]) => {
      setConfig(cfg)
      setWorkingDirectory(cfg.workingDirectory || '')
      setSandboxEnabled(cfg.sandboxEnabled)

      // Resolve acpPath: prefer config, fall back to auto-detected vibe-acp
      const acpPath = cfg.acpPath || setupStatus.vibAcpPath || null

      if (!setupStatus.vibAcpFound && !cfg.acpPath) {
        // vibe-acp not found and not manually configured — show setup
        setSetupRequired(true)
        return
      }

      if (acpPath) {
        // Auto-save discovered path if config didn't have one
        if (!cfg.acpPath && setupStatus.vibAcpPath) {
          window.api.config.save({ ...cfg, acpPath: setupStatus.vibAcpPath })
        }
        startAcp(acpPath)
      }
    })
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for ACP events
  useEffect(() => {
    const unsub = window.api.acp.onEvent(handleAcpEvent)
    return unsub
  }, [handleAcpEvent])

  // Apply theme
  useEffect(() => {
    const isDark = theme === 'auto'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
      : theme === 'dark'
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light')
  }, [theme])

  const handleSetupComplete = useCallback(
    async (vibAcpPath: string) => {
      setSetupRequired(false)
      const cfg = await window.api.config.get()
      // Persist path
      await window.api.config.save({ ...cfg, acpPath: vibAcpPath })
      startAcp(vibAcpPath)
    },
    [startAcp]
  )

  const handleSetupSkip = useCallback(() => {
    setSetupRequired(false)
  }, [])

  const handleApproveTool = useCallback((requestId: string, optionId: string) => {
    window.api.acp.approveTool(requestId, optionId)
    setPendingToolApprovals((prev) => prev.filter((r) => r.requestId !== requestId))
  }, [])

  const handleRejectTool = useCallback((requestId: string) => {
    window.api.acp.approveTool(requestId, 'reject_once')
    setPendingToolApprovals((prev) => prev.filter((r) => r.requestId !== requestId))
  }, [])

  const handleApproveAllTools = useCallback(() => {
    pendingToolApprovals.forEach((r) => {
      window.api.acp.approveTool(r.requestId, 'allow_once')
    })
    setPendingToolApprovals([])
  }, [pendingToolApprovals])

  const handleRejectAllTools = useCallback(() => {
    pendingToolApprovals.forEach((r) => {
      window.api.acp.approveTool(r.requestId, 'reject_once')
    })
    setPendingToolApprovals([])
  }, [pendingToolApprovals])

  return (
    <div className={`app-root ${theme === 'auto' ? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : theme}`}>
      <Header />
      <div className="app-body">
        <Sidebar />
        <main className="app-main">
          {currentView === 'chat' && <ChatView />}
          {currentView === 'settings' && <SettingsView />}
          {currentView === 'stats' && <StatsView />}
        </main>
      </div>

      {setupRequired && (
        <SetupView onComplete={handleSetupComplete} onSkip={handleSetupSkip} />
      )}

      {pendingQuestion && (
        <div className="tool-approval-bar">
          <div className="tool-approval-request agent-question">
            <div className="tool-approval-info" style={{ flexDirection: 'column', alignItems: 'flex-start', gap: 8 }}>
              <span className="agent-question-text">{pendingQuestion.question}</span>
              <div className="tool-approval-actions" style={{ flexWrap: 'wrap' }}>
                {pendingQuestion.choices.length > 0
                  ? pendingQuestion.choices.map((choice) => (
                      <button
                        key={choice}
                        className="tool-approval-btn tool-approval-allow-once"
                        onClick={() => {
                          window.api.acp.answerQuestion(pendingQuestion.requestId, choice).catch(console.error)
                          setPendingQuestion(null)
                        }}
                      >
                        {choice}
                      </button>
                    ))
                  : (
                    <button
                      className="tool-approval-btn tool-approval-allow-once"
                      onClick={() => {
                        window.api.acp.answerQuestion(pendingQuestion.requestId, 'ok').catch(console.error)
                        setPendingQuestion(null)
                      }}
                    >
                      OK
                    </button>
                  )
                }
              </div>
            </div>
          </div>
        </div>
      )}

      {pendingToolApprovals.length > 0 && (
        <div className="tool-approval-bar">
          {pendingToolApprovals.slice(0, 1).map((req) => {
            const params = req.params as Record<string, unknown>
            const input = params?.input as Record<string, unknown> | undefined
            const funcObj = params?.function as Record<string, unknown> | undefined
            const toolCall = params?.tool_call as Record<string, unknown> | undefined
            const toolName = (
              params?.resolvedToolName ??
              toolCall?.tool_name ??
              params?.name ??
              params?.tool ??
              funcObj?.name ??
              input?.name ??
              input?.tool ??
              req.method.replace('session/', '').replace('tool/', '')
            ) as string
            const displayName = toolName.length > 40 ? toolName.slice(0, 40) + '…' : toolName
            return (
              <div key={req.requestId} className="tool-approval-request">
                <div className="tool-approval-info">
                  <span className="tool-approval-icon">
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
                    </svg>
                  </span>
                  <span className="tool-approval-name">{t(tr, 'allowToolQuestion', { tool: displayName })}</span>
                </div>
                <div className="tool-approval-actions">
                  <button
                    className="tool-approval-btn tool-approval-allow-once"
                    onClick={() => handleApproveTool(req.requestId, 'allow_once')}
                    title={tr.allowOnce}
                  >
                    {tr.allowOnce}
                  </button>
                  <button
                    className="tool-approval-btn tool-approval-allow-always"
                    onClick={() => handleApproveTool(req.requestId, 'allow_always')}
                    title={tr.alwaysAllow}
                  >
                    {tr.alwaysAllow}
                  </button>
                  <button
                    className="tool-approval-btn tool-approval-deny"
                    onClick={() => handleRejectTool(req.requestId)}
                    title={tr.dontAllow}
                  >
                    {tr.dontAllow}
                  </button>
                </div>
              </div>
            )
          })}
          {pendingToolApprovals.length > 1 && (
            <div className="tool-approval-queue-hint">
              {tr.morePending.replace('{{count}}', String(pendingToolApprovals.length - 1))} &nbsp;
              <button className="tool-approval-btn tool-approval-allow-once" onClick={handleApproveAllTools}>{tr.allowAll}</button>
              <button className="tool-approval-btn tool-approval-deny" onClick={handleRejectAllTools}>{tr.denyAll}</button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
