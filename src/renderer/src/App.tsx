import React, { useEffect, useRef, useCallback, useState } from 'react'
import { useStore } from './store'
import type { AcpHistoryMessage } from './store'
import { useTranslation, t } from './i18n'
import type { AppConfig, ConfigOption, ToolCallDiff } from './types'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import SettingsView from './components/SettingsView'
import StatsView from './components/StatsView'
import HistoryView from './components/HistoryView'
import SetupView from './components/SetupView'
import Dock from './components/Dock'

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

interface AcpEvent {
  type: string
  data: Record<string, unknown>
}

interface StreamingState {
  msgId: string
  thinkingBlockId: string | null
  /** ACP messageId of the currently streamed agent message */
  acpMsgId: string | null
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
    setCurrentMode,
    setAvailableModes,
    setAvailableCommands,
    setConfigOptions,
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
    applySessionHistory,
    setPendingNewSession,
    setSessionStarting,
    currentSessionId,
    removeFromMessageQueue,
    addUserMessage,
    updateSession,
    markSessionBusy,
    markSessionIdle,
  } = useStore()

  const tr = useTranslation(config.language)

  // Per-session streaming state: localSessionId → streaming info
  const streamingState = useRef<Map<string, StreamingState>>(new Map())
  const [setupRequired, setSetupRequired] = useState(false)
  const [pendingToolApprovals, setPendingToolApprovals] = useState<
    Array<{ requestId: string; method: string; params: Record<string, unknown> }>
  >([])

  const [pendingQuestion, setPendingQuestion] = useState<{
    requestId: string
    question: string
    choices: string[]
  } | null>(null)

  const handleAcpEvent = useCallback(
    (event: AcpEvent) => {
      const d = event.data as Record<string, unknown>
      switch (event.type) {
        case 'InitializationComplete': {
          setAcpStatus('connected')
          setAcpError(null)
          const store = useStore.getState()
          // Prime one session so configOptions (model/mode/thinking) are
          // available before the first message. Guard against the flows where
          // InputBar/loadAcpSession are already creating their own session.
          if (!store.sessionStarting && !store.pendingLoadSessionId) {
            const primeCwd = store.config.workingDirectory || '.'
            window.api.acp.newSession(primeCwd).catch(console.error)
          }
          break
        }

        case 'SessionsList': {
          const sessions = d.sessions as Array<{
            sessionId: string
            title: string
            cwd: string
            updatedAt: string
          }>
          importAcpSessions(sessions)
          break
        }

        case 'SessionInitialized': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const modes = (d.modes as Array<{ id: string; name: string }>) ?? []
          const configOptions = (d.configOptions as ConfigOption[]) ?? []
          const sessionTitle = d.sessionTitle as string | undefined
          const currentModeId = d.currentModeId as string | undefined

          // While loading a specific historical session, ignore the priming
          // session that acp:start() creates internally.
          if (store.pendingLoadSessionId && acpSessId !== store.pendingLoadSessionId) {
            break
          }
          // Chips reflect only the session that is (or becomes) current —
          // never a background session (per-session model isolation).
          if (configOptions.length > 0) store.setConfigOptions(configOptions)
          store.setAvailableModes(modes)
          if (currentModeId) store.setCurrentMode(currentModeId)

          const existing = store.getSessionByAcpId(acpSessId)
          if (existing) {
            // Session already in list — make it current, update name if available
            store.setCurrentSessionId(existing.id)
            if (sessionTitle) store.updateSession(existing.id, { name: sessionTitle })
          } else {
            const localSessId = store.currentSessionId
            const localSess = localSessId
              ? store.sessions.find((s) => s.id === localSessId)
              : null
            if (localSess && !localSess.acpSessionId) {
              // Link the current draft to the new ACP session
              linkAcpSession(localSess.id, acpSessId)
              if (sessionTitle) store.updateSession(localSess.id, { name: sessionTitle })
            } else if (localSess && localSess.messages.length === 0) {
              // Empty draft bound to a stale priming session — re-bind it
              linkAcpSession(localSess.id, acpSessId)
              if (sessionTitle) store.updateSession(localSess.id, { name: sessionTitle })
            } else {
              const newSess = store.addSession()
              linkAcpSession(newSess.id, acpSessId)
              if (sessionTitle) store.updateSession(newSess.id, { name: sessionTitle })
            }
          }
          setSessionStarting(false)
          // A session with a recorded model/mode/thinking wins over the
          // CLI-reported defaults (sessions without their own turns would
          // otherwise fall back to the global active_model).
          const cur = useStore.getState().currentSession()
          if (cur?.acpSessionId === acpSessId) {
            void useStore.getState().syncSessionChips(cur.id)
          }
          break
        }

        case 'SessionHistory': {
          const store = useStore.getState()
          const messages = d.messages as AcpHistoryMessage[]
          const title = d.title as string | undefined
          if (messages) {
            store.applySessionHistory(d.sessionId as string, messages, title)
          }
          break
        }

        case 'SessionLoadProgress': {
          const store = useStore.getState()
          store.setSessionLoadProgress({
            acpSessionId: d.sessionId as string,
            count: d.count as number,
          })
          break
        }

        case 'SessionInfoUpdate': {
          const store = useStore.getState()
          const title = d.title as string | undefined
          const local = store.getSessionByAcpId(d.sessionId as string)
          if (local && title) {
            store.updateSession(local.id, { name: title, updatedAt: Date.now() })
          }
          break
        }

        case 'UsageUpdate': {
          const store = useStore.getState()
          const local = store.getSessionByAcpId(d.sessionId as string)
          if (!local) break
          const cost = d.cost as { amount: number; currency: string } | undefined
          store.setUsage(local.id, {
            contextUsed: (d.contextUsed as number) ?? 0,
            contextSize: (d.contextSize as number) ?? 0,
            cost: cost?.amount,
            currency: cost?.currency,
            steps: d.steps as number | undefined,
            promptTokens: d.promptTokens as number | undefined,
            completionTokens: d.completionTokens as number | undefined,
            cachedTokens: d.cachedTokens as number | undefined,
            totalTokens: d.totalTokens as number | undefined,
            tokensPerSecond: d.tokensPerSecond as number | undefined,
            lastTurnDuration: d.lastTurnDuration as number | undefined,
            lastTurnTotalTokens: d.lastTurnTotalTokens as number | undefined,
          })
          break
        }

        case 'ConfigOptionsUpdate': {
          const store = useStore.getState()
          const options = d.configOptions as ConfigOption[] | undefined
          const updSessId = d.sessionId as string | undefined
          const active = store.activeAcpSessionId
          // Only the active session may move the chips (per-session isolation).
          const isCurrent = !updSessId || !active || updSessId === active
          if (options && options.length > 0 && isCurrent) {
            store.setConfigOptions(options)
            // Record the reported values on the local session so switching
            // back restores them (per-session mode/thinking isolation).
            const target = updSessId ? store.getSessionByAcpId(updSessId) : store.currentSession()
            if (target) {
              const modelOpt = options.find((o) => o.id === 'model')
              const modeOpt = options.find((o) => o.id === 'mode')
              const thinkOpt = options.find((o) => o.id === 'thinking')
              const patch: { lastModel?: string; lastMode?: string; lastThinking?: string } = {}
              if (modelOpt && typeof modelOpt.currentValue === 'string') patch.lastModel = modelOpt.currentValue
              if (modeOpt && typeof modeOpt.currentValue === 'string') patch.lastMode = modeOpt.currentValue
              if (thinkOpt && typeof thinkOpt.currentValue === 'string') patch.lastThinking = thinkOpt.currentValue
              if (Object.keys(patch).length > 0) store.updateSession(target.id, patch)
            }
          }
          const modeChanged = d.modeChanged as string | undefined
          if (modeChanged && isCurrent) store.setCurrentMode(modeChanged)
          break
        }

        case 'MessageChunk': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          if (!localId) break
          if (store.pendingLoadSessionId !== acpSessId) markSessionBusy(localId)
          const text = d.text as string
          const acpMsgId = d.messageId as string | undefined
          let state = streamingState.current.get(localId)
          // A new messageId from the agent means a new agent message
          if (state && acpMsgId && state.acpMsgId && acpMsgId !== state.acpMsgId) {
            store.finalizeAiMessage(localId, state.msgId)
            state = undefined
          }
          if (!state) {
            const msgId = store.startAiMessage(localId)
            state = { msgId, thinkingBlockId: null, acpMsgId: acpMsgId ?? null }
            streamingState.current.set(localId, state)
          }
          store.appendChunk(localId, state.msgId, text)
          break
        }

        case 'ThinkingChunk': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          const blockId = (d.blockId as string) ?? 'thinking'
          if (!localId) break
          if (store.pendingLoadSessionId !== acpSessId) markSessionBusy(localId)
          let state = streamingState.current.get(localId)
          if (!state) {
            const msgId = store.startAiMessage(localId)
            state = { msgId, thinkingBlockId: null, acpMsgId: null }
            streamingState.current.set(localId, state)
          }
          if (state.thinkingBlockId !== blockId) {
            state.thinkingBlockId = blockId
            store.startThinking(localId, state.msgId, blockId)
          }
          store.appendThinkingChunk(localId, state.msgId, blockId, d.text as string)
          break
        }

        case 'ThinkingComplete': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          const blockId = (d.blockId as string) ?? 'thinking'
          const state = localId ? streamingState.current.get(localId) : null
          if (localId && state) {
            store.finalizeThinking(localId, state.msgId, blockId)
            state.thinkingBlockId = null
          }
          break
        }

        case 'PromptComplete': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          const state = localId ? streamingState.current.get(localId) : null
          if (localId && state) {
            store.finalizeAiMessage(localId, state.msgId)
            streamingState.current.delete(localId)
          }
          if (localId) markSessionIdle(localId)
          setAcpStatus('connected')

          // Process message queue — send the next queued message
          const queue = store.messageQueue
          if (queue.length > 0 && localId) {
            const nextMsg = queue[0]
            store.removeFromMessageQueue(nextMsg.id)
            store.addUserMessage(nextMsg.content)
            const sess = store.sessions.find((s) => s.id === localId)
            const promptText = sess?.contextSummary
              ? `${sess.contextSummary}\n\nUser: ${nextMsg.content}`
              : nextMsg.content
            if (store.pendingLoadSessionId !== acpSessId) markSessionBusy(localId)
            window.api.acp.sendPrompt(promptText, acpSessId).catch((err: Error) => {
              setAcpError(err.message)
            })
          }
          break
        }

        case 'ToolCall': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          if (!localId) break
          // Model-change checkpoints are CLI bookkeeping, not agent activity+�u���T
          // they must never flip the session into the busy state.
          const isCheckpoint = String(d.toolCallId ?? '').startsWith('checkpoint-')
          if (!isCheckpoint && store.pendingLoadSessionId !== acpSessId) markSessionBusy(localId)
          // Ensure there's an active AI message to attach tool calls to
          if (!streamingState.current.has(localId)) {
            const msgId = store.startAiMessage(localId)
            streamingState.current.set(localId, {
              msgId,
              thinkingBlockId: null,
              acpMsgId: null,
            })
          }
          updateToolCall(localId, {
            id: d.toolCallId as string,
            title: (d.title as string) ?? '',
            kind: (d.kind as string) ?? '',
            status: (d.status as 'running' | 'done' | 'error') ?? 'running',
            input: d.input as Record<string, unknown> | undefined,
            diffs: d.diffs as ToolCallDiff[] | undefined,
          })
          break
        }

        case 'ToolCallUpdate': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          if (localId) {
            mergeToolCall(localId, d.toolCallId as string, {
              status: (d.status as 'running' | 'done' | 'error') ?? 'running',
              output: d.output as string | undefined,
              diffs: d.diffs as ToolCallDiff[] | undefined,
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
          streamingState.current.forEach(({ msgId }, localId) => {
            finalizeAiMessage(localId, msgId)
          })
          streamingState.current.clear()
          break

        case 'Disconnected':
          setAcpStatus('disconnected')
          useStore.setState({ activeAcpSessionId: null })
          streamingState.current.clear()
          break

        case 'DebugLog':
        case 'Plan':
          break

        case 'AvailableCommandsUpdate': {
          const commands = d.commands as Array<{
            name: string
            description: string
            hint?: string
          }>
          setAvailableCommands(commands)
          break
        }
      }
    },
    [
      setAcpStatus,
      setAcpError,
      setCurrentModel,
      setCurrentMode,
      setAvailableModes,
      setAvailableCommands,
      setConfigOptions,
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
      applySessionHistory,
      setPendingNewSession,
      setSessionStarting,
      currentSessionId,
      removeFromMessageQueue,
      addUserMessage,
      updateSession,
      markSessionBusy,
      markSessionIdle,
    ],
  )

  const startAcp = useCallback(
    (acpPath: string) => {
      setAcpStatus('connecting')
      window.api.acp
        .init(acpPath)
        .then(() => {
          setSessionStarting(false)
        })
        .catch((err: Error) => {
          setAcpStatus('error')
          setAcpError(err.message)
        })
    },
    [setAcpStatus, setAcpError, setSessionStarting],
  )

  // Load config and bootstrap
  useEffect(() => {
    Promise.all([window.api.config.get(), window.api.setup.check()]).then(
      ([cfgRaw, setupStatusRaw]) => {
        const setupStatus = setupStatusRaw as SetupStatus
        const cfg = cfgRaw as AppConfig
        setConfig(cfg)
        setWorkingDirectory(cfg.workingDirectory || '')
        setSandboxEnabled(cfg.sandboxEnabled)

        // Resolve acpPath: prefer config, fall back to auto-detected vibe-acp
        const acpPath = cfg.acpPath || setupStatus.vibAcpPath || null

        if (!setupStatus.vibAcpFound && !cfg.acpPath) {
          setSetupRequired(true)
          return
        }

        if (acpPath) {
          if (!cfg.acpPath && setupStatus.vibAcpPath) {
            window.api.config.save({ ...cfg, acpPath: setupStatus.vibAcpPath })
          }
          startAcp(acpPath)
        }
      },
    )
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for ACP events
  useEffect(() => {
    const unsub = window.api.acp.onEvent(handleAcpEvent)
    return unsub
  }, [handleAcpEvent])

  // Apply theme
  useEffect(() => {
    const isDark =
      theme === 'auto'
        ? window.matchMedia('(prefers-color-scheme: dark)').matches
        : theme === 'dark'
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light')
  }, [theme])

  const handleSetupComplete = useCallback(
    async (vibAcpPath: string) => {
      setSetupRequired(false)
      const cfg = (await window.api.config.get()) as AppConfig
      await window.api.config.save({ ...cfg, acpPath: vibAcpPath })
      startAcp(vibAcpPath)
    },
    [startAcp],
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
    <div
      className={`app-root ${
        theme === 'auto'
          ? window.matchMedia('(prefers-color-scheme: dark)').matches
            ? 'dark'
            : 'light'
          : theme
      }`}
    >
      <Header />
      <div className="app-body">
        <Sidebar />
        <main className="app-main">
          {currentView === 'chat' && <ChatView />}
          {currentView === 'settings' && <SettingsView />}
          {currentView === 'stats' && <StatsView />}
          {currentView === 'history' && <HistoryView />}
        </main>
        <Dock />
      </div>

      {setupRequired && <SetupView onComplete={handleSetupComplete} onSkip={handleSetupSkip} />}

      {pendingQuestion && (
        <div className="tool-approval-bar">
          <div className="tool-approval-request agent-question">
            <div
              className="tool-approval-info"
              style={{ flexDirection: 'column', alignItems: 'flex-start', gap: 8 }}
            >
              <span className="agent-question-text">{pendingQuestion.question}</span>
              <div className="tool-approval-actions" style={{ flexWrap: 'wrap' }}>
                {pendingQuestion.choices.length > 0
                  ? pendingQuestion.choices.map((choice) => (
                      <button
                        key={choice}
                        className="tool-approval-btn tool-approval-allow-once"
                        onClick={() => {
                          window.api.acp
                            .answerQuestion(pendingQuestion.requestId, choice)
                            .catch(console.error)
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
                        window.api.acp
                          .answerQuestion(pendingQuestion.requestId, 'ok')
                          .catch(console.error)
                        setPendingQuestion(null)
                      }}
                    >
                      OK
                    </button>
                  )}
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
            const displayName =
              toolName.length > 40 ? toolName.slice(0, 40) + '…' : toolName
            return (
              <div key={req.requestId} className="tool-approval-request">
                <div className="tool-approval-info">
                  <span className="tool-approval-icon">
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      width="14"
                      height="14"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <circle cx="12" cy="12" r="10" />
                      <line x1="12" y1="8" x2="12" y2="12" />
                      <line x1="12" y1="16" x2="12.01" y2="16" />
                    </svg>
                  </span>
                  <span className="tool-approval-name">
                    {t(tr, 'allowToolQuestion', { tool: displayName })}
                  </span>
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
              {tr.morePending.replace('{{count}}', String(pendingToolApprovals.length - 1))}{' '}
              <button className="tool-approval-btn tool-approval-allow-once" onClick={handleApproveAllTools}>
                {tr.allowAll}
              </button>
              <button className="tool-approval-btn tool-approval-deny" onClick={handleRejectAllTools}>
                {tr.denyAll}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
