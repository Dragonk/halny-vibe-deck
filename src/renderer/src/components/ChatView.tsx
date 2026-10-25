import React, { useEffect, useMemo, useRef } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import MessageBubble from './MessageBubble'
import InputBar from './InputBar'
import type { PlanEntry } from '../types'

const EMPTY_PLAN: PlanEntry[] = []

export default function ChatView(): React.JSX.Element {
  const { currentSession, streamingSessions, busySessions, stoppingSessions, currentSessionId, acpStatus, acpError, config, pendingNewSession, pendingLoadSessionId, sessionLoadProgress } =
    useStore()
  const tr = useTranslation(config.language)
  const session = pendingNewSession ? null : currentSession()
  const scrollRef = useRef<HTMLDivElement>(null)

  const isCurrentSessionStreaming = currentSessionId != null && streamingSessions.includes(currentSessionId)
  const isCurrentSessionBusy = currentSessionId != null && busySessions.includes(currentSessionId)
  const isResuming = !!session && (session.loading || (!!session.acpSessionId && pendingLoadSessionId === session.acpSessionId))
  const resumeCount = session && sessionLoadProgress && session.acpSessionId === sessionLoadProgress.acpSessionId ? sessionLoadProgress.count : null
  const showAgentBusy = isCurrentSessionBusy && !isCurrentSessionStreaming
  // v0.10.5: stop request in flight - say Stopping... instead of Agent working.
  const isCurrentSessionStopping = currentSessionId != null && stoppingSessions.includes(currentSessionId)
  // v0.11.1: agent plan/todo checklist for the viewed session (live + replay).
  const sessionPlans = useStore((s) => s.sessionPlans)
  const planEntries = useMemo(
    () => (session?.acpSessionId ? sessionPlans[session.acpSessionId] : undefined) ?? EMPTY_PLAN,
    [sessionPlans, session?.acpSessionId],
  )

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [session?.messages.length, isCurrentSessionStreaming])

  return (
    <div className="chat-view-wrap">
      <div className="chat-messages" ref={scrollRef}>
        <div className="chat-inner">
          {!session || (session.messages.length === 0 && !isResuming) ? (
            <div className="chat-empty">
              <svg xmlns="http://www.w3.org/2000/svg" width="95" height="68" viewBox="0 0 212.121 151.515" version="1.1" shapeRendering="crispEdges" className="chat-empty-icon">
                <rect fill="var(--yellowbright-500)" x="30.303001" y="0" width="30.302999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--yellowbright-500)" x="151.515" y="0" width="30.302999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--yellow-500)" x="30.303001" y="30.303001" width="60.605999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--yellow-500)" x="121.21201" y="30.303001" width="60.605999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--orangebright-500)" x="30.303001" y="60.606003" width="151.515" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--orange-500)" x="30.303001" y="90.908997" width="30.302999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--orange-500)" x="90.908997" y="90.908997" width="30.302999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--orange-500)" x="151.515" y="90.908997" width="30.302999" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--red-700)" x="0" y="121.21201" width="90.908997" height="30.302999" strokeWidth="0"/>
                <rect fill="var(--red-700)" x="121.21201" y="121.21201" width="90.908997" height="30.302999" strokeWidth="0"/>
              </svg>
              <div className="chat-empty-title">
                {pendingNewSession ? tr.newChat : session ? tr.startConversation : tr.appName}
              </div>
              <div className="chat-empty-subtitle">
                {acpStatus === 'connected'
                  ? tr.agentReady
                  : acpStatus === 'connecting'
                    ? tr.connectingToAgent
                    : acpStatus === 'error'
                      ? tr.agentConnectionError
                      : tr.configureAcpInSettings}
              </div>
              {acpStatus === 'error' && acpError && (
                <div className="chat-error-banner">
                  <span>⚠</span> {acpError}
                </div>
              )}
            </div>
          ) : (
            <div className="message-list">
              {session.messages.map((msg, idx) => (
                <MessageBubble
                  key={msg.id}
                  message={msg}
                  isStreaming={
                    isCurrentSessionStreaming && idx === session.messages.length - 1
                  }
                  isAgentBusy={
                    isCurrentSessionBusy && msg.sender === 'ai' && idx === session.messages.length - 1
                  }
                  acpSessionId={session.acpSessionId}
                />
              ))}
              {planEntries.length > 0 && (
                <div className="plan-card">
                  <div className="plan-card-header">
                    <span className="plan-card-title">{tr.planLabel}</span>
                    <span className="plan-card-count">
                      {planEntries.filter((e) => e.status === 'completed').length}/{planEntries.length}
                    </span>
                  </div>
                  {planEntries.map((entry, pIdx) => (
                    <div key={pIdx} className={'plan-row plan-' + entry.status}>
                      <span className="plan-icon">
                        {entry.status === 'completed' ? '✓' : entry.status === 'in_progress' ? '◐' : '○'}
                      </span>
                      <span className="plan-content">{entry.content}</span>
                    </div>
                  ))}
                </div>
              )}
              {showAgentBusy && !isResuming && (
                <div className="agent-busy-row">
                  <span className="agent-busy-dots"><span /><span /><span /></span>
                  <span className="steps-shimmer">
                    {isCurrentSessionStopping ? tr.stoppingLabel : tr.agentWorkingLabel}
                  </span>
                </div>
              )}
            </div>
          )}
          {isResuming && (
            <div className="agent-busy-row">
              <span className="agent-busy-spinner" />
              {tr.resumingSessionLabel}
              {resumeCount != null ? ' (' + resumeCount + ')' : ''}
            </div>
          )}
        </div>
      </div>
      <InputBar config={config} />
    </div>
  )
}
