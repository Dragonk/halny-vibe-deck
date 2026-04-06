import React, { useState, useRef, useCallback, useEffect } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import type { VibeSession } from '../types'

const MIN_SIDEBAR_WIDTH = 120
const DEFAULT_SIDEBAR_WIDTH = 220
const MAX_SIDEBAR_WIDTH = 400

export default function Sidebar(): React.JSX.Element {
  const {
    sessions,
    currentSessionId,
    setCurrentSessionId,
    currentView,
    setCurrentView,
    pendingNewSession,
    setPendingNewSession,
    busySessions,
    config,
    loadVibeSessionMessages,
    fetchContextSummary,
    setCurrentModel
  } = useStore()
  const tr = useTranslation(config.language)
  const [hoveredSessionId, setHoveredSessionId] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const [vibeSessions, setVibeSessions] = useState<VibeSession[]>([])
  const sidebarRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    window.api.stats.vibeSessions().then(setVibeSessions).catch(() => setVibeSessions([]))
  }, [])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    setIsResizing(true)
  }, [])

  useEffect(() => {
    if (!isResizing) return

    const handleMouseMove = (e: MouseEvent): void => {
      const newWidth = Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, e.clientX))
      setSidebarWidth(newWidth)
    }

    const handleMouseUp = (): void => {
      setIsResizing(false)
    }

    document.addEventListener('mousemove', handleMouseMove)
    document.addEventListener('mouseup', handleMouseUp)

    return () => {
      document.removeEventListener('mousemove', handleMouseMove)
      document.removeEventListener('mouseup', handleMouseUp)
    }
  }, [isResizing])

  const handleNewSession = (): void => {
    setPendingNewSession(true)
    setCurrentSessionId(null)
    setCurrentView('chat')
  }

  const handleSelectSession = async (id: string): Promise<void> => {
    setCurrentSessionId(id)
    setPendingNewSession(false)
    setCurrentView('chat')

    const session = sessions.find(s => s.id === id)

    if (session?.acpSessionId) {
      // Load session metadata to get active model and agent
      const statsApi = window.api.stats as unknown as { vibeSessionMetadata: (id: string) => Promise<{ activeModel: string | null; agentProfile: string | null }> }
      const metadata = await statsApi.vibeSessionMetadata(session.acpSessionId)
      if (metadata.activeModel) {
        setCurrentModel(metadata.activeModel)
      }

      if (session.messages.length === 0) {
        await loadVibeSessionMessages(id)
        if (session.messages.length > 0) {
          await fetchContextSummary(id)
        }
      }
    }
  }

  const sortedSessions = [...sessions].sort((a, b) => b.lastActive - a.lastActive)
  const displayedSessions = config.sidebarSessionsLimit && config.sidebarSessionsLimit > 0
    ? sortedSessions.slice(0, config.sidebarSessionsLimit)
    : sortedSessions

  const getSessionTimestamp = (session: typeof sessions[0]): number => {
    if (session.acpSessionId) {
      const vibeSess = vibeSessions.find(v => v.sessionId === session.acpSessionId)
      if (vibeSess?.startTime) {
        return new Date(vibeSess.startTime).getTime()
      }
    }
    return session.startTime || session.lastActive || session.createdAt
  }

  const getSessionCwd = (session: typeof sessions[0]): string | undefined => {
    if (session.acpSessionId) {
      const vibeSess = vibeSessions.find(v => v.sessionId === session.acpSessionId)
      if (vibeSess?.environment?.workingDirectory) {
        return vibeSess.environment.workingDirectory.split(/[/\\]/).pop()
      }
    }
    return session.workingDirectory?.split(/[/\\]/).pop()
  }

  return (
    <aside
      ref={sidebarRef}
      className={`sidebar ${isResizing ? 'sidebar-resizing' : ''}`}
      style={{ width: sidebarWidth }}
    >
      <div className="sidebar-header">
        <button
          className={`new-session-btn ${pendingNewSession ? 'active' : ''}`}
          onClick={handleNewSession}
        >
          <span className="new-session-icon">+</span>
          {tr.newSession}
        </button>
      </div>

      <div className="sidebar-sessions-label">{tr.sessions}</div>

      <nav className="session-list">
        {displayedSessions.length === 0 && (
          <span className="session-empty">{tr.noSessionsYet}</span>
        )}
        {displayedSessions.map((session) => {
          const isActive = !pendingNewSession && session.id === currentSessionId && currentView === 'chat'
          const isBusy = busySessions.includes(session.id)
          const isHovered = hoveredSessionId === session.id
          const timestamp = getSessionTimestamp(session)
          const sessionDate = new Date(timestamp)
          const dateStr = isNaN(sessionDate.getTime())
            ? ''
            : sessionDate.toLocaleDateString(undefined, { year: 'numeric', month: '2-digit', day: '2-digit' })
          const cwd = getSessionCwd(session)
          return (
            <button
              key={session.id}
              className={`session-item ${isActive ? 'active' : ''}`}
              onClick={() => handleSelectSession(session.id)}
              onMouseEnter={() => setHoveredSessionId(session.id)}
              onMouseLeave={() => setHoveredSessionId(null)}
            >
              <div className="session-item-content">
                <div className="session-item-row">
                  {isBusy && (
                    <svg className="session-busy-spinner" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                      <path d="M21 12a9 9 0 1 1-6.219-8.56" />
                    </svg>
                  )}
                  <span className="session-name">{session.name}</span>
                </div>
                <div className="session-item-meta">
                  <span className="session-date">{dateStr}</span>
                  {cwd && <span className="session-cwd">{cwd}</span>}
                </div>
              </div>
              {isBusy && isHovered && session.acpSessionId && (
                <button
                  className="session-cancel-btn"
                  title="Cancel"
                  onClick={(e) => {
                    e.stopPropagation()
                    window.api.acp.cancelSession(session.acpSessionId!).catch(console.error)
                  }}
                >
                  ✕
                </button>
              )}
            </button>
          )
        })}
      </nav>

      <nav className="sidebar-nav">
        <button
          className={`sidebar-nav-item ${currentView === 'stats' ? 'active' : ''}`}
          onClick={() => setCurrentView('stats')}
        >
          {tr.statistics}
        </button>
        <button
          className={`sidebar-nav-item ${currentView === 'settings' ? 'active' : ''}`}
          onClick={() => setCurrentView('settings')}
        >
          {tr.settings}
        </button>
      </nav>

      <div className="sidebar-resize-handle" onMouseDown={handleMouseDown} />
    </aside>
  )
}
