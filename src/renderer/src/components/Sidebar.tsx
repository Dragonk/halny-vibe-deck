import React, { useState, useRef, useCallback, useEffect } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import type { Session } from '../types'

const MIN_SIDEBAR_WIDTH = 120
const DEFAULT_SIDEBAR_WIDTH = 220
const MAX_SIDEBAR_WIDTH = 400

interface ProjectGroup {
  key: string
  label: string
  fullPath: string
  sessions: Session[]
  newest: number
}

export default function Sidebar(): React.JSX.Element {
  const {
    sessions,
    currentSessionId,
    setCurrentSessionId,
    currentView,
    setCurrentView,
    pendingNewSession,
    setPendingNewSession,
    setSessionStarting,
    busySessions,
    config,
    loadAcpSession,
    collapsedProjects,
    toggleProjectCollapse,
  } = useStore()
  const tr = useTranslation(config.language)
  const [hoveredSessionId, setHoveredSessionId] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const sidebarRef = useRef<HTMLDivElement>(null)

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
    // Reuse an existing empty draft (e.g. the priming session) if any
    const emptyDraft = sessions.find(
      (s) =>
        s.messages.length === 0 &&
        !busySessions.includes(s.id) &&
        !s.loading &&
        s.acpSessionId,
    )
    if (emptyDraft) {
      setCurrentSessionId(emptyDraft.id)
      setPendingNewSession(false)
      setCurrentView('chat')
      return
    }
    setPendingNewSession(true)
    setSessionStarting(false)
    setCurrentSessionId(null)
    setCurrentView('chat')
  }

  const handleSelectSession = async (id: string): Promise<void> => {
    setCurrentSessionId(id)
    setPendingNewSession(false)
    setCurrentView('chat')

    const session = sessions.find((s) => s.id === id)
    if (session?.acpSessionId && session.messages.length === 0 && !session.loading) {
      // Real resume via ACP session/load (replays the full history)
      await loadAcpSession(id)
    }
  }

  // Newest first, always — then group by working directory
  const sortedSessions = [...sessions].sort(
    (a, b) => (b.updatedAt ?? b.lastActive) - (a.updatedAt ?? a.lastActive),
  )
  const displayedSessions =
    config.sidebarSessionsLimit && config.sidebarSessionsLimit > 0
      ? sortedSessions.slice(0, config.sidebarSessionsLimit)
      : sortedSessions

  const groups = new Map<string, ProjectGroup>()
  for (const session of displayedSessions) {
    const wd = session.workingDirectory ?? ''
    const key = wd || '__none__'
    if (!groups.has(key)) {
      groups.set(key, {
        key,
        label: wd ? wd.split(/[/\\]/).filter(Boolean).pop() ?? wd : tr.noProject,
        fullPath: wd,
        sessions: [],
        newest: 0,
      })
    }
    const group = groups.get(key)!
    group.sessions.push(session)
    const ts = session.updatedAt ?? session.lastActive
    if (ts > group.newest) group.newest = ts
  }
  const sortedGroups = [...groups.values()].sort((a, b) => b.newest - a.newest)

  const renderSession = (session: Session) => {
    const isActive =
      !pendingNewSession && session.id === currentSessionId && currentView === 'chat'
    const isBusy = busySessions.includes(session.id)
    const isHovered = hoveredSessionId === session.id
    const timestamp = session.updatedAt ?? session.lastActive ?? session.createdAt
    const sessionDate = new Date(timestamp)
    const dateStr = isNaN(sessionDate.getTime())
      ? ''
      : sessionDate.toLocaleDateString(undefined, {
          year: '2-digit',
          month: '2-digit',
          day: '2-digit',
        })
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
              <svg
                className="session-busy-spinner"
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
              >
                <path d="M21 12a9 9 0 1 1-6.219-8.56" />
              </svg>
            )}
            {session.loading && <span className="session-loading-dot">⟳</span>}
            <span className="session-name">{session.name}</span>
          </div>
          <div className="session-item-meta">
            <span className="session-date">{dateStr}</span>
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

      <nav className="session-list session-list-projects">
        {displayedSessions.length === 0 && (
          <span className="session-empty">{tr.noSessionsYet}</span>
        )}
        {sortedGroups.map((group) => {
          const collapsed = collapsedProjects.includes(group.key)
          return (
            <div key={group.key} className="project-group">
              <button
                className="project-group-header"
                onClick={() => toggleProjectCollapse(group.key)}
                title={group.fullPath || tr.noProject}
              >
                <span className={`project-chevron ${collapsed ? '' : 'open'}`}>▸</span>
                <span className="project-name" title={group.fullPath}>
                  {group.label}
                </span>
                <span className="project-count">{group.sessions.length}</span>
              </button>
              {!collapsed && group.sessions.map(renderSession)}
            </div>
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
