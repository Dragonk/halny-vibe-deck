import React, { useEffect, useRef, useCallback, useState } from 'react'
import { useStore } from './store'
import type { AcpHistoryMessage } from './store'
import { useTranslation, t, resolveLanguage } from './i18n'
import type { AppConfig, ConfigOption, ToolCallDiff } from './types'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import SettingsView from './components/SettingsView'
import StatsView from './components/StatsView'
import HistoryView from './components/HistoryView'
import SetupView from './components/SetupView'
import Dock from './components/Dock'
import { isWebMode } from './webBridge'

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
    mobileSidebarOpen,
    setMobileSidebarOpen,
  } = useStore()

  const tr = useTranslation(config.language)
  const updateInfo = useStore((s) => s.updateInfo)
  const dismissUpdateBanner = useStore((s) => s.dismissUpdateBanner)

  const handleUpdateApp = useCallback(async () => {
    setUpdateBusy(true)
    setUpdateError(null)
    const res = await useStore.getState().updateAppNow()
    setUpdateBusy(false)
    if (!res.ok && res.error) setUpdateError(res.error)
  }, [])

  const handleUpdateCli = useCallback(async () => {
    setUpdateBusy(true)
    setUpdateError(null)
    const res = await useStore.getState().updateCliNow()
    setUpdateBusy(false)
    if (!res.ok && res.error) setUpdateError(res.error)
  }, [])

  // v0.7.2: in web mode remember the resolved UI language so the pre-auth
  // login page can greet the user in their language on the next visit.
  useEffect(() => {
    if (!isWebMode()) return
    try {
      localStorage.setItem('nexusLang', resolveLanguage(config.language))
    } catch {
      /* private mode: storage unavailable */
    }
  }, [config.language])

  // Per-session streaming state: localSessionId → streaming info
  const streamingState = useRef<Map<string, StreamingState>>(new Map())
  const [setupRequired, setSetupRequired] = useState(false)
  const [updateBusy, setUpdateBusy] = useState(false)
  const [updateError, setUpdateError] = useState<string | null>(null)
  const [pendingToolApprovals, setPendingToolApprovals] = useState<
    Array<{ requestId: string; method: string; params: Record<string, unknown>; remoteId: string | null }>
  >([])

  const [pendingQuestion, setPendingQuestion] = useState<{
    requestId: string
    question: string
    choices: string[]
    remoteId: string | null
  } | null>(null)

  const handleAcpEvent = useCallback(
    (event: AcpEvent) => {
      const d = event.data as Record<string, unknown>
      switch (event.type) {
        case 'InitializationComplete': {
          const rid = (d.remoteId as string | undefined) ?? null
          setAcpStatus('connected')
          setAcpError(null)
          const store = useStore.getState()
          // Prime one session on the LOCAL agent so configOptions
          // (model/mode/thinking) are available before the first message.
          // v0.7.0: remote workers never prime - they start on demand and
          // the renderer creates their sessions explicitly.
          if (!rid && !store.sessionStarting && !store.pendingLoadSessionId) {
            const primeCwd = store.config.workingDirectory || '.'
            window.api.acp.newSession(primeCwd).catch(console.error)
          }
          break
        }

        case 'SessionsList': {
          // v0.7.0: remote workers list their own sessions on start - those
          // arrive via refreshRemoteData instead; never import as local ones.
          if (d.remoteId) break
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
          const rid = (d.remoteId as string | undefined) ?? null
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
            // v0.7.0: only a session belonging to the emitting worker may
            // become current (multi-worker safety).
            if ((existing.remoteId ?? null) !== rid) break
            // Session already in list — make it current, update name if available
            store.setCurrentSessionId(existing.id)
            if (sessionTitle) store.updateSession(existing.id, { name: sessionTitle })
          } else {
            const localSessId = store.currentSessionId
            const localSess = localSessId
              ? store.sessions.find((s) => s.id === localSessId)
              : null
            // v0.7.0: a draft can only link to a session from the same worker
            // (local draft ↔ local agent, remote draft ↔ its own worker).
            if (localSess && (localSess.remoteId ?? null) === rid && !localSess.acpSessionId) {
              // Link the current draft to the new ACP session
              linkAcpSession(localSess.id, acpSessId)
              if (sessionTitle) store.updateSession(localSess.id, { name: sessionTitle })
            } else if (localSess && (localSess.remoteId ?? null) === rid && localSess.messages.length === 0) {
              // Empty draft bound to a stale priming session — re-bind it
              linkAcpSession(localSess.id, acpSessId)
              if (sessionTitle) store.updateSession(localSess.id, { name: sessionTitle })
            } else if (!rid) {
              const newSess = store.addSession()
              linkAcpSession(newSess.id, acpSessId)
              if (sessionTitle) store.updateSession(newSess.id, { name: sessionTitle })
            } else {
              // Remote session with no local draft — leave it alone; the next
              // remote refresh imports it into the sidebar.
              break
            }
          }
          setSessionStarting(false)
          // A session with a recorded model/mode/thinking wins over the
          // CLI-reported defaults (sessions without their own turns would
          // otherwise fall back to the global active_model).
          const cur = useStore.getState().currentSession()
          if (cur?.acpSessionId === acpSessId) {
            useStore.getState().markSessionLive(cur.id)
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
            const rid = (d.remoteId as string | undefined) ?? null
            window.api.acp.sendPrompt(promptText, acpSessId, rid ?? undefined).catch((err: Error) => {
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
          const rid = (d.remoteId as string | undefined) ?? null
          setPendingToolApprovals((prev) => [...prev, { requestId, method, params, remoteId: rid }])
          break
        }

        case 'AskUserQuestion': {
          const requestId = d.requestId as string
          const question = d.question as string
          const choices = d.choices as string[]
          const rid = (d.remoteId as string | undefined) ?? null
          setPendingQuestion({ requestId, question, choices, remoteId: rid })
          break
        }

        case 'Error': {
          const rid = (d.remoteId as string | undefined) ?? null
          setAcpError(d.error as string)
          // v0.7.0: only the local agent drives the global status; a remote
          // worker failing must not mark the whole app disconnected.
          if (!rid) setAcpStatus('error')
          streamingState.current.forEach(({ msgId }, localId) => {
            finalizeAiMessage(localId, msgId)
          })
          streamingState.current.clear()
          break
        }

        case 'Disconnected': {
          const rid = (d.remoteId as string | undefined) ?? null
          if (rid) {
            // v0.7.0: a remote worker died - the local agent is unaffected.
            console.error('[acp] remote worker disconnected:', rid)
            break
          }
          setAcpStatus('disconnected')
          useStore.setState({ activeAcpSessionId: null })
          streamingState.current.clear()
          break
        }

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

  // v0.9.0: auto-close idle live sessions (config.autoCloseIdleMinutes,
  // default 60; 0 disables). Sending a message to a closed session revives
  // it through the regular resume path.
  useEffect(() => {
    const iv = window.setInterval(() => {
      const store = useStore.getState()
      const minutes = store.config.autoCloseIdleMinutes ?? 60
      if (!minutes || minutes <= 0) return
      const now = Date.now()
      for (const id of [...store.liveSessions]) {
        if (store.busySessions.includes(id)) continue
        const sess = store.sessions.find((s) => s.id === id)
        const ts = store.sessionActivity[id] ?? sess?.updatedAt ?? sess?.lastActive ?? 0
        if (ts > 0 && now - ts > minutes * 60000) store.closeSession(id)
      }
    }, 60000)
    return () => window.clearInterval(iv)
  }, [])

  // v0.9.0: look for app + CLI updates shortly after startup so the banner
  // appears without opening Settings. Silent - failures just keep the banner
  // hidden.
  useEffect(() => {
    const t = window.setTimeout(() => {
      void useStore.getState().checkAppUpdate()
      void useStore.getState().checkCliUpdate()
    }, 4000)
    return () => window.clearTimeout(t)
  }, [])

  // v0.6.0: poll remote SSH connections (status dots + remote session lists).
  // The store loads the config asynchronously after mount, so the effect must
  // re-run once the enabled connections become available - and whenever they
  // are edited in Settings - instead of waiting for the first 60 s tick.
  const remoteConnIds = useStore(
    (s) =>
      (s.config.remoteConnections ?? [])
        .filter((c) => c.enabled)
        .map((c) => c.id)
        .join(','),
  )
  useEffect(() => {
    void useStore.getState().refreshRemoteData()
    const iv = window.setInterval(() => {
      void useStore.getState().refreshRemoteData()
    }, 60000)
    return () => window.clearInterval(iv)
  }, [remoteConnIds])

  // v0.6.0: mobile gestures — swipe from the left edge reveals the sidebar,
  // swipe from the right edge opens the dock (touch devices / PWA).
  useEffect(() => {
    let startX = 0
    let startY = 0
    let viewW = window.innerWidth
    let fromEdge = false
    const onTouchStart = (e: TouchEvent): void => {
      if (e.touches.length !== 1) return
      const t = e.touches[0]
      startX = t.clientX
      startY = t.clientY
      viewW = window.innerWidth
      fromEdge = startX <= 36 || startX >= viewW - 36
    }
    const onTouchEnd = (e: TouchEvent): void => {
      if (!fromEdge) return
      fromEdge = false
      const t = e.changedTouches[0]
      const dx = t.clientX - startX
      const dy = Math.abs(t.clientY - startY)
      if (dy > 80) return
      if (startX <= 36 && dx > 60) useStore.getState().setMobileSidebarOpen(true)
      else if (startX >= viewW - 36 && dx < -60) useStore.getState().ensureDockOpen()
    }
    window.addEventListener('touchstart', onTouchStart, { passive: true })
    window.addEventListener('touchend', onTouchEnd, { passive: true })
    return () => {
      window.removeEventListener('touchstart', onTouchStart)
      window.removeEventListener('touchend', onTouchEnd)
    }
  }, [])

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
    const r = pendingToolApprovals.find((x) => x.requestId === requestId)
    window.api.acp.approveTool(requestId, optionId, r?.remoteId ?? undefined)
    setPendingToolApprovals((prev) => prev.filter((x) => x.requestId !== requestId))
  }, [pendingToolApprovals])

  const handleRejectTool = useCallback((requestId: string) => {
    const r = pendingToolApprovals.find((x) => x.requestId === requestId)
    window.api.acp.approveTool(requestId, 'reject_once', r?.remoteId ?? undefined)
    setPendingToolApprovals((prev) => prev.filter((x) => x.requestId !== requestId))
  }, [pendingToolApprovals])

  const handleApproveAllTools = useCallback(() => {
    pendingToolApprovals.forEach((r) => {
      window.api.acp.approveTool(r.requestId, 'allow_once', r.remoteId ?? undefined)
    })
    setPendingToolApprovals([])
  }, [pendingToolApprovals])

  const handleRejectAllTools = useCallback(() => {
    pendingToolApprovals.forEach((r) => {
      window.api.acp.approveTool(r.requestId, 'reject_once', r.remoteId ?? undefined)
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
      {!updateInfo.dismissed &&
        ((updateInfo.app?.updateAvailable ?? false) || (updateInfo.cli?.updateAvailable ?? false)) && (
          <div className="update-banner">
            <div className="update-banner-texts">
              {updateInfo.app?.updateAvailable && (
                <span>
                  {tr.updateBannerApp.replace('{{version}}', updateInfo.app.latestVersion ?? '')}
                </span>
              )}
              {updateInfo.cli?.updateAvailable && (
                <span>
                  {tr.updateBannerCli.replace('{{version}}', updateInfo.cli.latestVersion ?? '')}
                </span>
              )}
              {updateError && <span className="update-banner-error">{updateError}</span>}
            </div>
            <div className="update-banner-actions">
              {updateInfo.app?.updateAvailable && (
                <button className="update-banner-btn" disabled={updateBusy} onClick={handleUpdateApp}>
                  {tr.updateBannerAction}
                </button>
              )}
              {updateInfo.cli?.updateAvailable && (
                <button className="update-banner-btn" disabled={updateBusy} onClick={handleUpdateCli}>
                  {tr.updateBannerAction}
                </button>
              )}
              <button
                className="update-banner-dismiss"
                title={tr.updateBannerDismiss}
                onClick={dismissUpdateBanner}
              >
                ✕
              </button>
            </div>
          </div>
        )}
      {mobileSidebarOpen && (
        <div className="mobile-scrim" onClick={() => setMobileSidebarOpen(false)} />
      )}
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
                            .answerQuestion(pendingQuestion.requestId, choice, pendingQuestion.remoteId ?? undefined)
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
                          .answerQuestion(pendingQuestion.requestId, 'ok', pendingQuestion.remoteId ?? undefined)
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
