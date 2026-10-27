import React, { useEffect, useRef, useCallback, useState } from 'react'
import { useStore } from './store'
import type { AcpHistoryMessage } from './store'
import { useTranslation, t, resolveLanguage } from './i18n'
import type { AppConfig, ConfigOption, PlanEntry, SubagentInfo, ToolCallDiff } from './types'
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

  // v1.0.22: the busy flag set from streaming chunks can be STALE -
  // out-of-band chunks (the CLI's MCP warm-up warning arrives as an
  // agent_message_chunk right after session creation) mark a session
  // busy while NO turn exists, and the send button then stays a stop
  // button forever (a dead-end: the queue waits for a turn end that
  // never comes). The SERVER owns the truth (acp:busy-sessions) -
  // verify shortly after the last chunk and heal the flag when the
  // server reports no turn for the session.
  const busyVerifyTimers = useRef<Map<string, number>>(new Map())
  const armBusyVerify = useCallback((localId: string): void => {
    const prev = busyVerifyTimers.current.get(localId)
    if (prev) window.clearTimeout(prev)
    busyVerifyTimers.current.set(
      localId,
      window.setTimeout(() => {
        busyVerifyTimers.current.delete(localId)
        void (async () => {
          const st = useStore.getState()
          if (!st.busySessions.includes(localId)) return
          const sess = st.sessions.find((x) => x.id === localId)
          if (!sess?.acpSessionId) return
          try {
            const busy = (await window.api.acp.busySessions()) as Array<{ acpSessionId: string }>
            if (!busy.some((b) => b.acpSessionId === sess.acpSessionId)) {
              useStore.getState().markSessionIdle(localId)
            }
          } catch {
            /* server unreachable - keep the state */
          }
        })()
      }, 1500),
    )
  }, [])
  const [setupRequired, setSetupRequired] = useState(false)
  // v1.0.13: "press back again to exit" toast (PWA back guard)
  const [backToast, setBackToast] = useState(false)
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
              // v0.13.15: create a local session ONLY when this client has
              // no current session at all (first priming draft after boot).
              // When the user is viewing a resumed/busy session, an unknown
              // SessionInitialized (a priming replay, another client starting
              // a chat, a re-broadcast after worker restart) must never spawn
              // an extra sidebar entry - that is the duplicate-session bug.
              // The unknown ACP session is imported through SessionsList
              // instead, which dedupes by acpSessionId.
              // v1.0.1: the v0.13.15 guard inverted its own comment:
              // it broke exactly when this client had NO current session
              // (fresh install, or the pending new-session draft), so the
              // boot prime was never linked and the first prompt went to
              // an invisible session while the user message was dropped.
              // Create the local session when there is no current session;
              // only a session currently being viewed (localSess) or a
              // pending load is left alone (v0.13.15 duplicate-session
              // fix stays intact).
              if (localSess || store.pendingLoadSessionId) break
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
          if (store.pendingLoadSessionId !== acpSessId) {
            markSessionBusy(localId)
            armBusyVerify(localId)
          }
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
          // v0.13.7: remember which messageId this message carries. The split
          // check above compares against it, but it was never assigned here, so
          // once thinking or a tool call created the state first (acpMsgId null)
          // EVERY intermediate text of the turn merged into one giant bubble.
          state.acpMsgId = acpMsgId ?? state.acpMsgId
          break
        }

        case 'ThinkingChunk': {
          const acpSessId = d.sessionId as string
          const store = useStore.getState()
          const local = store.getSessionByAcpId(acpSessId)
          const localId = local?.id ?? store.currentSessionId
          const blockId = (d.blockId as string) ?? 'thinking'
          if (!localId) break
          if (store.pendingLoadSessionId !== acpSessId) {
            markSessionBusy(localId)
            armBusyVerify(localId)
          }
          let state = streamingState.current.get(localId)
          // v0.13.7: reasoning that arrives after a streamed text segment opens
          // the NEXT agent message - finalize the current one so the thought
          // block lands above its own text, not glued under the previous text.
          if (state && state.acpMsgId !== null) {
            store.finalizeAiMessage(localId, state.msgId)
            streamingState.current.delete(localId)
            state = undefined
          }
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
            // v1.0.21: deterministic id + echo payload - other clients
            // (phone) must see the queued message too.
            const qid = 'q-' + nextMsg.id
            store.addUserMessage(nextMsg.content, undefined, qid)
            const sess = store.sessions.find((s) => s.id === localId)
            const promptText = sess?.contextSummary
              ? `${sess.contextSummary}\n\nUser: ${nextMsg.content}`
              : nextMsg.content
            if (store.pendingLoadSessionId !== acpSessId) {
              markSessionBusy(localId)
              armBusyVerify(localId)
            }
            const rid = (d.remoteId as string | undefined) ?? null
            // v1.0.21: the worker may be DEAD here (a hung turn - e.g. an
            // MCP-OAuth login - was just stopped; the ssh worker died with
            // it). Deliver with the SAME recovery as the send-time
            // reviver: revive the worker, restore (or recreate) the CLI
            // session, then send - a queued slash command (/mcp login)
            // must reach the agent, not die as a raw error.
            void (async () => {
              try {
                await store.ensureAcpWorker(rid, sess?.workingDirectory || '')
                try {
                  await window.api.acp.sendPrompt(promptText, acpSessId, rid ?? undefined, undefined, {
                    id: qid,
                    text: nextMsg.content,
                  })
                } catch (sendErr) {
                  const m = String((sendErr as Error)?.message ?? sendErr)
                  if (!/not started|not running|No active session|Session not found|No target session/.test(m)) {
                    throw sendErr
                  }
                  // The CLI session is gone with the worker - restore it
                  // from disk (or recreate when the session never had
                  // history) and deliver.
                  await window.api.acp
                    .loadSession(acpSessId, sess?.workingDirectory || '.', rid ?? undefined)
                    .catch(() =>
                      window.api.acp.newSession(sess?.workingDirectory || '.', rid ?? undefined),
                    )
                  await window.api.acp.sendPrompt(promptText, acpSessId, rid ?? undefined, undefined, {
                    id: qid,
                    text: nextMsg.content,
                  })
                }
              } catch (err: unknown) {
                setAcpError((err as Error)?.message ?? String(err))
                if (store.pendingLoadSessionId !== acpSessId) markSessionIdle(localId)
              }
            })()
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
          if (!isCheckpoint && store.pendingLoadSessionId !== acpSessId) {
            markSessionBusy(localId)
            armBusyVerify(localId)
          }
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
            previousContent: (d.previousContent as string | null | undefined) ?? null,
            toolName: (d.toolName as string | null | undefined) ?? null,
            rawOutput: (d.rawOutput as Record<string, unknown> | null | undefined) ?? null,
            subagent: (d.subagent as SubagentInfo | null | undefined) ?? null,
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
              previousContent: (d.previousContent as string | null | undefined) ?? null,
              toolName: (d.toolName as string | null | undefined) ?? null,
              rawOutput: (d.rawOutput as Record<string, unknown> | null | undefined) ?? null,
            subagent: (d.subagent as SubagentInfo | null | undefined) ?? null,
            })
          }
          break
        }

        case 'UserMessage': {
          // v1.0.10: a prompt sent from ANY client (desktop, another
          // browser, the phone) appears in every client. The sending
          // client added it optimistically with the SAME id - recognized
          // here and skipped; everyone else inserts it fresh.
          const msgId = d.id as string
          const acpId = d.sessionId as string
          const st = useStore.getState()
          const sess = st.sessions.find((s) => s.acpSessionId === acpId)
          if (!sess || !msgId) break
          if (sess.messages.some((m) => m.id === msgId)) break
          const msg = {
            id: msgId,
            sender: 'user' as const,
            content: (d.text as string) ?? '',
            isComplete: true,
            timestamp: Date.now(),
            ...((d.attachments as Array<{ path: string; isDir: boolean; content: string }> | undefined)?.length
              ? { attachments: d.attachments as Array<{ path: string; isDir: boolean; content: string }> }
              : {}),
          }
          st.updateSession(sess.id, {
            messages: [...sess.messages, msg],
            lastActive: Date.now(),
            updatedAt: Date.now(),
            name:
              sess.messages.length === 0
                ? String(d.text ?? '').slice(0, 40) + (String(d.text ?? '').length > 40 ? '…' : '')
                : sess.name,
          })
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
          const store = useStore.getState()
          // v0.13.11: a dead worker must not leave its sessions stuck busy
          // with a spinning thinking indicator and a blocked input. Finalize
          // whatever was streaming for the affected worker, release its busy
          // flags and surface why the turn stopped. The next prompt revives
          // the worker and resumes the session in place (no duplicate).
          const isAffected = (id: string) => {
            const s = store.sessions.find((x) => x.id === id)
            if (!s) return !rid
            return rid ? s.remoteId === rid : !s.remoteId
          }
          let interrupted = false
          streamingState.current.forEach(({ msgId }, localId) => {
            if (!isAffected(localId)) return
            interrupted = true
            useStore.getState().finalizeAllThinking(localId, msgId)
            useStore.getState().completeToolCalls(localId, msgId)
            finalizeAiMessage(localId, msgId)
          })
          streamingState.current.forEach((_, localId) => {
            if (isAffected(localId)) streamingState.current.delete(localId)
          })
          let releasedBusy = false
          store.busySessions.forEach((busyId) => {
            if (!isAffected(busyId)) return
            releasedBusy = true
            useStore.getState().markSessionIdle(busyId)
          })
          if (rid) {
            // v0.7.0: a remote worker died - the local agent is unaffected.
            console.error('[acp] remote worker disconnected:', rid)
            if (interrupted || releasedBusy) {
              setAcpError('Remote agent worker disconnected - the turn was interrupted. Send your message again to resume this session.')
            }
            break
          }
          setAcpStatus('disconnected')
          useStore.setState({ activeAcpSessionId: null })
          if (interrupted || releasedBusy) {
            setAcpError('Agent process exited - the turn was interrupted. Send your message again to resume this session.')
          }
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
          // v1.0.13: after the worker is up, learn which sessions run a
          // turn RIGHT NOW (a reopened PWA must see running turns as
          // busy, not idle/"stalled") and load the current session's
          // history from disk.
          void useStore.getState().syncBusySessions()
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
        // v1.0.8: on the phone, boot into the SESSION LIST - not the
        // preview dock and not an auto-opened chat. The user starts a
        // session consciously or resumes one from the list.
        if (window.matchMedia('(max-width: 768px)').matches) {
          const st = useStore.getState()
          if (st.dockOpen) st.setDockOpen(false)
          if (!st.mobileSidebarOpen) st.setMobileSidebarOpen(true)
        }
        // v1.0.7: no session is opened at boot - populate the model/mode/
        // thinking chips straight from the vibe config so the user sees
        // what can be passed to the agent before any ACP connection.
        void useStore.getState().loadDefaultChips()

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

  // v1.0.9: the system back gesture/button (Android phone) pops ONE view
  // layer at a time: an open panel, then edit mode / preview tabs /
  // settings - exactly like a native app.
  // v1.0.13: the back gesture can never CLOSE the app on a single press:
  // with nothing left to pop a toast asks for a second press within 2 s
  // (the native-app pattern); only that confirmed double-back lets the
  // browser history leave the app.
  useEffect(() => {
    if (!isWebMode()) return
    // v1.0.18 (audit of the v1.0.14 failure): a layer pushed WITHOUT a
    // history entry is invisible to the Android back gesture - with an
    // empty history stack the PWA closes (or reloaded through the
    // /login 302 -> boot state). The documented PWA contract: forward
    // navigation pushes an entry (store.pushBack does it now), popstate
    // pops exactly one layer WITHOUT re-pushing (the browser already
    // consumed the entry), and a guard entry below the app + a
    // double-back window make the exit intentional:
    // back=preview, back=file list, ..., back=session list (boot),
    // back=toast, back again=app closes.
    window.history.replaceState({ nav: 0 }, '')
    window.history.pushState({ nav: 'guard' }, '')
    let lastBareBack = 0
    const onPop = (ev: PopStateEvent): void => {
      const st = useStore.getState()
      // v1.0.18: reconcile by the LANDED history entry - a manual
      // panel close (menu/swipe) consumed its layer without a
      // popstate, so the back gesture may land on an entry whose
      // layer is already gone; pop only layers pushed AFTER the
      // landed one.
      const landed = (ev.state as { nav?: number | string } | null)?.nav
      if (typeof landed === 'number' && landed > 0) {
        const stack = st.backStack
        let stale = 0
        for (let i = stack.length - 1; i >= 0; i--) {
          if (stack[i].id > landed) stale++
          else break
        }
        if (stale > 0) {
          useStore.setState({ backStack: stack.slice(0, stack.length - stale) })
        }
        return
      }
      if (st.backStack.length > 0) {
        // Landed on the guard/base entry: the top layer's state was
        // skipped past - run its restore (one layer per gesture).
        st.popBack()
        return
      }
      const now = Date.now()
      if (now - lastBareBack < 2000) {
        // Confirmed double-back from the boot state: close the app.
        window.history.back()
        return
      }
      lastBareBack = now
      setBackToast(true)
      window.setTimeout(() => setBackToast(false), 2000)
      // Re-arm the guard so this press never exits the app.
      window.history.pushState({ nav: 'guard' }, '')
    }
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // v1.0.13: after a WS reconnect (server restart, network blip) re-sync
  // the client state so the view is never stale.
  // v1.0.17: the resync now pulls the FULL state (busy/live flags +
  // on-disk history of every loaded session + SSH lists).
  useEffect(() => {
    if (!isWebMode()) return
    const onOpen = (): void => void useStore.getState().resyncAfterReconnect()
    window.addEventListener('nexus-ws-open', onOpen)
    return () => window.removeEventListener('nexus-ws-open', onOpen)
  }, [])

  // v1.0.17: the PWA waking up from the background (page was frozen,
  // Android kept it alive) - events streamed while away were missed
  // even with a live WS. Every return to visibility pulls the full
  // state again; closing the PWA never stops anything server-side.
  useEffect(() => {
    if (!isWebMode()) return
    const onVis = (): void => {
      if (document.visibilityState === 'visible') {
        void useStore.getState().resyncAfterReconnect()
      }
    }
    document.addEventListener('visibilitychange', onVis)
    return () => document.removeEventListener('visibilitychange', onVis)
  }, [])

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
      {/* v1.0.13: native-style "press back again to exit" toast */}
      {backToast && <div className="back-toast">{tr.backExitToast}</div>}
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
            const rawName = (
              params?.resolvedToolName ??
              toolCall?.tool_name ??
              params?.name ??
              params?.tool ??
              funcObj?.name ??
              input?.name ??
              input?.tool ??
              req.method.replace('session/', '').replace('tool/', '')
            ) as string
            // ACP titles often look like "bash: cd ... && ..." - keep only the
            // short tool name in the question and show the command separately
            // in a monospace chip, so long commands never stretch the bar.
            const titleMatch = rawName.match(/^([A-Za-z0-9_.-]+):\s*([\s\S]+)$/)
            const toolName = titleMatch ? titleMatch[1] : rawName
            const displayName =
              toolName.length > 24 ? toolName.slice(0, 24) + '…' : toolName
            const command = (
              input?.command ??
              input?.cmd ??
              input?.script ??
              input?.code ??
              (titleMatch ? titleMatch[2] : '')
            ) as string
            const commandPreview =
              typeof command === 'string' && command.trim().length > 0
                ? command.trim().slice(0, 400)
                : ''
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
                {commandPreview && (
                  <div className="tool-approval-input">{commandPreview}</div>
                )}
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
