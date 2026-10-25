import React, { useEffect, useRef, useCallback, useState } from 'react'
import { useStore } from './store'
import type { AcpHistoryMessage } from './store'
import { useTranslation, t, resolveLanguage } from './i18n'
import type { AppConfig, ConfigOption, PlanEntry, ToolCallDiff } from './types'
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
  const updateProgress = useStore((s) => s.updateProgress)
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
          // v0.10.7: priming is owned by the main process (acp:init primes
          // once per worker lifetime). The renderer used to call newSession
          // here as well, so every window start / web F5 created a duplicate
          // ACP session and could hijack the current view in other clients.
          setAcpStatus('connected')
          setAcpError(null)
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
          // session that the main-process boot prime (acp:init) creates.
          if (store.pendingLoadSessionId && acpSessId !== store.pendingLoadSessionId) {
            break
          }
          // Chips reflect only the session that is (or becomes) current —
          // never a background session (per-session model isolation).
          // v0.11.1: also snapshot the option list per session - switching
          // back must restore the thinking levels of this session's model.
          if (configOptions.length > 0) {
            store.setSessionConfigOptions(acpSessId, configOptions)
            store.setConfigOptions(configOptions)
          }
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
          // v0.10.2: when the main process already applied the recorded
          // config inside session/new|load, skip syncSessionChips - its
          // async set_config_option could race the first prompt and hang
          // the CLI turn.
          const appliedConfig = d.appliedConfig as
            | { model?: string; mode?: string; thinking?: string }
            | undefined
          const cur = useStore.getState().currentSession()
          if (cur?.acpSessionId === acpSessId) {
            useStore.getState().markSessionLive(cur.id)
            if (appliedConfig) {
              if (appliedConfig.model) store.setCurrentModel(appliedConfig.model)
              if (appliedConfig.mode) store.setCurrentMode(appliedConfig.mode)
              if (appliedConfig.thinking) store.setCurrentThinking(appliedConfig.thinking)
            } else {
              // A session with a recorded model/mode/thinking wins over the
              // CLI-reported defaults (sessions without their own turns would
              // otherwise fall back to the global active_model).
              void useStore.getState().syncSessionChips(cur.id)
            }
          }
          break
        }

        case 'SessionHistory': {
          const store = useStore.getState()
          const messages = d.messages as AcpHistoryMessage[]
          const title = d.title as string | undefined
          if (messages) {
            store.applySessionHistory(d.sessionId as string, messages, title)
            // v0.11.1: the main process assembles the replayed plan state -
            // restore it so the checklist survives a session resume.
            store.setSessionPlan(d.sessionId as string, (d.plan as PlanEntry[] | undefined) ?? [])
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
          const cur = store.currentSession()
          // v0.10.6: chips may only move when the update targets the session
          // this client is viewing. The old guard (activeAcpSessionId) let
          // updates for other sessions hijack the chips and dropped updates
          // for the viewed session while it was in lazy preview.
          const isCurrent = !updSessId || (!!cur && cur.acpSessionId === updSessId)
          // Record the reported values on the target session in every client
          // (switching back restores them), not only when it is the view.
          const target = updSessId ? store.getSessionByAcpId(updSessId) : cur
          if (options && options.length > 0) {
            // v0.11.1: keep a per-session snapshot (option lists are model specific).
            if (updSessId) store.setSessionConfigOptions(updSessId, options)
            const modelOpt = options.find((o) => o.id === 'model')
            const modeOpt = options.find((o) => o.id === 'mode')
            const thinkOpt = options.find((o) => o.id === 'thinking')
            if (target) {
              const patch: { lastModel?: string; lastMode?: string; lastThinking?: string } = {}
              if (modelOpt && typeof modelOpt.currentValue === 'string') patch.lastModel = modelOpt.currentValue
              if (modeOpt && typeof modeOpt.currentValue === 'string') patch.lastMode = modeOpt.currentValue
              if (thinkOpt && typeof thinkOpt.currentValue === 'string') patch.lastThinking = thinkOpt.currentValue
              if (Object.keys(patch).length > 0) store.updateSession(target.id, patch)
            }
            if (isCurrent) {
              store.setConfigOptions(options)
              // v0.10.6: the chips read the current* store state (not
              // option.currentValue) - sync them from the confirmed options
              // so a change made in another client reaches this one too.
              if (modelOpt && typeof modelOpt.currentValue === 'string') store.setCurrentModel(modelOpt.currentValue)
              if (modeOpt && typeof modeOpt.currentValue === 'string') store.setCurrentMode(modeOpt.currentValue)
              if (thinkOpt && typeof thinkOpt.currentValue === 'string') store.setCurrentThinking(thinkOpt.currentValue)
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
          // v0.10.2: the CLI never emits agent_thought_end - once the answer
          // text streams, all thinking blocks of this message are done.
          store.finalizeAllThinking(localId, state.msgId)
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
            // v0.10.2: close open thinking blocks and stuck tool calls -
            // the CLI does not always emit their end updates.
            store.finalizeAllThinking(localId, state.msgId)
            store.completeToolCalls(localId, state.msgId)
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
          const tcState = streamingState.current.get(localId)
          if (tcState) {
            // v0.10.2: tool activity means thinking is over (no thought_end).
            store.finalizeAllThinking(localId, tcState.msgId)
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
            // v0.10.2: on error, close open thinking blocks and tool calls.
            useStore.getState().finalizeAllThinking(localId, msgId)
            useStore.getState().completeToolCalls(localId, msgId)
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

        case 'Plan': {
          const store = useStore.getState()
          const entries = (d.entries as PlanEntry[] | undefined) ?? []
          if (d.sessionId) store.setSessionPlan(d.sessionId as string, entries)
          break
        }

        case 'DebugLog':
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

  // v0.11.0: app self-update download/install progress (banner + Settings).
  useEffect(() => {
    const off = window.api.updates.onProgress((p) => {
      useStore.getState().setUpdateProgress(
        p as { phase: string; received: number; total: number | null; percent: number | null },
      )
    })
    return off
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

  // v0.10.4: mobile gestures — swipes work anywhere on the screen, not just
  // at the edges. Swipe right reveals the sidebar, swipe left reveals the
  // dock; when a panel is on top the opposite swipe hides it (left hides the
  // session list, right hides the previews). Touch devices / PWA.
  useEffect(() => {
    let startX = 0
    let startY = 0
    let gestureOk = false
    const onTouchStart = (e: TouchEvent): void => {
      if (e.touches.length !== 1) return
      if (!window.matchMedia('(max-width: 768px)').matches) {
        gestureOk = false
        return
      }
      const t = e.touches[0]
      startX = t.clientX
      startY = t.clientY
      const el = e.target as HTMLElement | null
      // Swipes start anywhere; skip touches that begin in text controls or
      // horizontally scrollable strips so typing and tab swiping are never
      // misread as gestures.
      gestureOk =
        !!el &&
        !el.closest(
          'input, textarea, [contenteditable="true"], pre, code, table, .dock-tabs',
        )
    }
    const onTouchEnd = (e: TouchEvent): void => {
      if (!gestureOk || e.changedTouches.length !== 1) return
      gestureOk = false
      const t = e.changedTouches[0]
      const dx = t.clientX - startX
      const dy = t.clientY - startY
      // A deliberate swipe: long enough and clearly horizontal (vertical
      // scrolling must not trigger it).
      if (Math.abs(dx) < 60 || Math.abs(dx) < Math.abs(dy) * 1.5) return
      const st = useStore.getState()
      const sidebarOpen = st.mobileSidebarOpen
      const dockVisible = st.dockOpen && st.currentView === 'chat'
      if (dx > 0) {
        // Swipe right: push the dock away, otherwise reveal the sidebar.
        if (dockVisible) st.setDockOpen(false)
        else if (!sidebarOpen) st.setMobileSidebarOpen(true)
      } else {
        // Swipe left: hide the session list, otherwise reveal the dock.
        if (sidebarOpen) st.setMobileSidebarOpen(false)
        else if (st.currentView === 'chat') st.ensureDockOpen()
      }
    }
    window.addEventListener('touchstart', onTouchStart, { passive: true })
    window.addEventListener('touchend', onTouchEnd, { passive: true })
    return () => {
      window.removeEventListener('touchstart', onTouchStart)
      window.removeEventListener('touchend', onTouchEnd)
    }
  }, [])

  // v0.10.4: keep the input card above the on-screen keyboard. The layout
  // height tracks the visual viewport (--vv-h, set here in JS), so when the
  // keyboard appears the whole app - including the input card - shrinks to
  // the visible area and the user sees what they are typing. The viewport
  // meta (interactive-widget=resizes-content) covers browsers that resize
  // the layout viewport natively; this handles the rest (e.g. iOS Safari).
  useEffect(() => {
    const vv = window.visualViewport
    if (!vv) return
    const onVv = (): void => {
      document.documentElement.style.setProperty('--vv-h', Math.round(vv.height) + 'px')
    }
    vv.addEventListener('resize', onVv)
    vv.addEventListener('scroll', onVv)
    onVv()
    return () => {
      vv.removeEventListener('resize', onVv)
      vv.removeEventListener('scroll', onVv)
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
              {updateBusy && updateProgress && (
                <span className="update-banner-progress">
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
              {updateBusy && updateProgress?.phase === 'download' && (updateProgress.percent ?? 0) > 0 && (
                <div className="update-progress-track">
                  <div className="update-progress-fill" style={{ width: (updateProgress.percent ?? 0) + '%' }} />
                </div>
              )}
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
