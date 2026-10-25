import React, { useState, useRef, useCallback, useEffect } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import { isWebMode } from '../webBridge'
import type { Session, RemoteConnection } from '../types'

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

interface DeleteTarget {
  kind: 'session' | 'project'
  /** acpSessionId (fallback: local session id) for sessions, project key for projects */
  key: string
  label: string
  /** project working directory ('' = sessions without a project) */
  wd: string
  count: number
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
    hideSession,
    hideProject,
    remoteStatuses,
    startRemoteSession,
    mobileSidebarOpen,
    setMobileSidebarOpen,
    liveSessions,
    closeSession,
  } = useStore()
  const tr = useTranslation(config.language)
  const [hoveredSessionId, setHoveredSessionId] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; target: DeleteTarget } | null>(null)
  const [confirmTarget, setConfirmTarget] = useState<DeleteTarget | null>(null)
  const sidebarRef = useRef<HTMLDivElement>(null)

  const openCtx = useCallback((e: React.MouseEvent, target: DeleteTarget): void => {
    e.preventDefault()
    e.stopPropagation()
    setCtxMenu({ x: e.clientX, y: e.clientY, target })
  }, [])

  // Long-press (touch) opens the same context menu as right-click (v0.6.0).
  const longPressTimer = useRef<number | null>(null)
  const suppressClick = useRef(false)
  const clearLongPress = useCallback((): void => {
    if (longPressTimer.current !== null) {
      window.clearTimeout(longPressTimer.current)
      longPressTimer.current = null
    }
  }, [])

  // v0.6.0: called on touchMove/touchEnd. When the long-press already fired,
  // prevent the default so Chromium does NOT synthesize mousedown/mouseup/click
  // after the finger is lifted - those synthesized events would immediately
  // close the context menu the long-press just opened.
  const endLongPress = useCallback((e: React.TouchEvent): void => {
    clearLongPress()
    if (suppressClick.current) e.preventDefault()
  }, [clearLongPress])
  const sessionTarget = (session: Session): DeleteTarget => ({
    kind: 'session',
    key: session.acpSessionId ?? session.id,
    label: session.name,
    wd: session.workingDirectory ?? '',
    count: 1,
  })
  const startLongPress = useCallback(
    (e: React.TouchEvent, target: DeleteTarget): void => {
      const t = e.touches[0]
      if (!t) return
      const x = t.clientX
      const y = t.clientY
      clearLongPress()
      longPressTimer.current = window.setTimeout(() => {
        longPressTimer.current = null
        suppressClick.current = true
        setCtxMenu({ x, y, target })
      }, 550)
    },
    [clearLongPress],
  )

  // Close the context menu on any click outside of it
  useEffect(() => {
    if (!ctxMenu) return
    const close = (e: MouseEvent): void => {
    const el = e.target as HTMLElement | null
    if (el && el.closest('.ctx-menu')) return
    setCtxMenu(null)
  }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [ctxMenu])

  const executeDelete = useCallback((target: DeleteTarget): void => {
    // Only hides entries in the UI — nothing is deleted from disk, so
    // statistics and the unified session store stay intact.
    if (target.kind === 'session') hideSession(target.key)
    else hideProject(target.wd)
    setConfirmTarget(null)
    setCtxMenu(null)
  }, [hideSession, hideProject])

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

  // v0.8.1: per-project + buttons start a session directly in that
  // project (vibe must run in the right location); the bottom
  // "Choose folder..." button picks an arbitrary local folder.
  const startLocalDraft = (cwd?: string): void => {
    setMobileSidebarOpen(false)
    if (cwd) {
      const store = useStore.getState()
      store.addRecentFolder(cwd)
      const draft = store.addSession(cwd)
      setPendingNewSession(true)
      setSessionStarting(false)
      setCurrentSessionId(draft.id)
      setCurrentView('chat')
      return
    }
    // Reuse an existing empty draft (e.g. the priming session) if any
    const emptyDraft = sessions.find(
      (s) =>
        s.messages.length === 0 &&
        !busySessions.includes(s.id) &&
        !s.loading &&
        !s.remoteId &&
        liveSessions.includes(s.id) &&
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

  const chooseFolderForSession = async (): Promise<void> => {
    const folder = await window.api.dialog.openFolder()
    if (!folder) return
    startLocalDraft(folder)
  }

  const handleSelectSession = async (id: string): Promise<void> => {
    setCurrentSessionId(id)
    setPendingNewSession(false)
    setCurrentView('chat')
    setMobileSidebarOpen(false)

    const session = sessions.find((s) => s.id === id)
    if (!session?.acpSessionId) return
    if (liveSessions.includes(id)) {
      // v0.9.0: only LIVE sessions resume over ACP (loadAcpSession ensures
      // the right worker is alive). A live session without messages is a
      // freshly primed draft - a real load fills its history.
      if (session.messages.length === 0 && !session.loading) {
        await loadAcpSession(id)
      } else {
        useStore.setState({ activeAcpSessionId: session.acpSessionId })
        void useStore.getState().syncSessionChips(id)
      }
      return
    }
    // v0.9.0 lazy resume: clicking a session only reads its history from
    // disk (no vibe-acp start). ACP wakes up when the user consciously
    // sends a message in this session.
    if (session.messages.length === 0 && !session.loading) {
      await useStore.getState().previewSession(id)
    }
  }

  // v0.9.0: live (running) sessions first, then newest first
  const sortedSessions = [...sessions].sort((a, b) => {
    const la = liveSessions.includes(a.id) ? 1 : 0
    const lb = liveSessions.includes(b.id) ? 1 : 0
    if (la !== lb) return lb - la
    return (b.updatedAt ?? b.lastActive) - (a.updatedAt ?? a.lastActive)
  })
  const hiddenSessions = config.hiddenSessions ?? []
  const hiddenProjects = config.hiddenProjects ?? []
  const visibleSessions = sortedSessions.filter((s) => {
    if (hiddenSessions.includes(s.acpSessionId ?? s.id)) return false
    const wd = s.workingDirectory ?? ''
    if (wd !== '' && hiddenProjects.includes(wd)) return false
    return true
  })
  const displayedSessions =
    config.sidebarSessionsLimit && config.sidebarSessionsLimit > 0
      ? visibleSessions.slice(0, config.sidebarSessionsLimit)
      : visibleSessions

  const remoteConns: RemoteConnection[] = (config.remoteConnections ?? []).filter(
    (c) => c.enabled,
  )
  const localDisplayed = displayedSessions.filter((s) => !s.remoteId)

  const groups = new Map<string, ProjectGroup>()
  for (const session of localDisplayed) {
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
    const isLive = liveSessions.includes(session.id)
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
        onClick={() => {
          if (suppressClick.current) {
            suppressClick.current = false
            return
          }
          handleSelectSession(session.id)
        }}
        onMouseEnter={() => setHoveredSessionId(session.id)}
        onMouseLeave={() => setHoveredSessionId(null)}
        onTouchStart={(e) => startLongPress(e, sessionTarget(session))}
        onTouchMove={endLongPress}
        onTouchEnd={endLongPress}
        onContextMenu={(e) => openCtx(e, sessionTarget(session))}
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
            {session.remoteId && <span className="remote-badge-mini" title="SSH">📍</span>}
            {isLive && <span className="session-live-dot" title={tr.sessionActiveLabel} />}
            <span className="session-name">{session.name}</span>
          </div>
          <div className="session-item-meta">
            <span className="session-date">{dateStr}</span>
          </div>
        </div>
        {isBusy && isHovered && session.acpSessionId && (
          <button
            className="session-cancel-btn"
            title={tr.cancelLabel}
            onClick={(e) => {
              e.stopPropagation()
              window.api.acp
                .cancelSession(session.acpSessionId!, session.remoteId ?? undefined)
                .catch(console.error)
            }}
          >
            ✕
          </button>
        )}
        {!isBusy && isLive && isHovered && (
          <button
            className="session-cancel-btn"
            title={tr.closeSessionLabel}
            onClick={(e) => {
              e.stopPropagation()
              closeSession(session.id)
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
      className={`sidebar ${isResizing ? 'sidebar-resizing' : ''} ${mobileSidebarOpen ? 'mobile-open' : ''}`}
      style={{ width: sidebarWidth }}
    >
      <button
        className="mobile-panel-close"
        title={tr.closePanelLabel}
        onClick={() => setMobileSidebarOpen(false)}
      >
        ✕
      </button>

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
                onContextMenu={(e) =>
                  openCtx(e, {
                    kind: 'project',
                    key: group.key,
                    label: group.label,
                    wd: group.fullPath,
                    count: group.sessions.length,
                  })
                }
                title={group.fullPath || tr.noProject}
              >
                <span className={`project-chevron ${collapsed ? '' : 'open'}`}>▸</span>
                <span className="project-name" title={group.fullPath}>
                  {group.label}
                </span>
                <span className="project-count">{group.sessions.length}</span>
                <span
                  className="pgh-plus"
                  title={tr.newSessionHere}
                  onClick={(e) => {
                    e.stopPropagation()
                    startLocalDraft(group.fullPath || undefined)
                  }}
                >
                  ＋
                </span>
              </button>
              {!collapsed && group.sessions.map(renderSession)}
            </div>
          )
        })}
        {!isWebMode() && (
          <button
            className="nsd-folder-pick"
            onClick={() => void chooseFolderForSession()}
          >
            📂 {tr.newSessionChooseFolder}
          </button>
        )}
        {remoteConns.map((conn) => {
          const collapsed = collapsedProjects.includes('remote:' + conn.id)
          const st = remoteStatuses[conn.id]
          const connSessions = displayedSessions.filter((s) => s.remoteId === conn.id)
          return (
            <div key={'remote-' + conn.id} className="project-group remote-project-group">
              <button
                className="project-group-header"
                onClick={() => toggleProjectCollapse('remote:' + conn.id)}
                title={conn.sshCmd}
              >
                <span className={`project-chevron ${collapsed ? '' : 'open'}`}>▸</span>
                <span className="remote-badge">📍</span>
                <span className="project-name">{conn.name}</span>
                <span
                  className={`remote-dot ${st?.ok ? 'ok' : 'down'}`}
                  title={st?.ok ? tr.remoteConnUp : tr.remoteConnDown}
                />
              </button>
              {!collapsed && (
                <>
                  {conn.locations.map((loc) => (
                    <button
                      key={conn.id + '-loc-' + loc}
                      className="session-item remote-location-item"
                      title={loc}
                      onClick={() => {
                        setMobileSidebarOpen(false)
                        void startRemoteSession(conn.id, loc)
                      }}
                    >
                      <div className="session-item-content">
                        <div className="session-item-row">
                          <span className="remote-loc-icon">📁</span>
                          <span className="session-name">
                            {loc.split(/[/\\]/).filter(Boolean).pop() || loc}
                          </span>
                        </div>
                      </div>
                    </button>
                  ))}
                  {(() => {
                    // v0.7.0: group the connection's sessions per project cwd
                    const wdGroups = new Map<string, typeof connSessions>()
                    for (const s of connSessions) {
                      const wd = s.workingDirectory || ''
                      const list = wdGroups.get(wd)
                      if (list) list.push(s)
                      else wdGroups.set(wd, [s])
                    }
                    return Array.from(wdGroups.entries()).map(([wd, list]) => {
                      const grpKey = 'remote:' + conn.id + ':' + wd
                      const grpCollapsed = collapsedProjects.includes(grpKey)
                      const label =
                        wd
                          ? wd.split(/[\\/]/).filter(Boolean).pop() || wd
                          : tr.noProject
                      return (
                        <div key={grpKey} className="remote-cwd-group">
                          <button
                            className="project-group-header remote-cwd-header"
                            onClick={() => toggleProjectCollapse(grpKey)}
                            title={wd || tr.noProject}
                          >
                            <span className={`project-chevron ${grpCollapsed ? '' : 'open'}`}>▸</span>
                            <span className="project-name">{label}</span>
                            <span className="project-count">{list.length}</span>
                            <span
                              className="pgh-plus"
                              title={tr.newSessionHere}
                              onClick={(e) => {
                                e.stopPropagation()
                                void startRemoteSession(conn.id, wd)
                              }}
                            >
                              ＋
                            </span>
                          </button>
                          {!grpCollapsed && list.map((s) => renderSession(s))}
                        </div>
                      )
                    })
                  })()}
                </>
              )}
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
          className={`sidebar-nav-item ${currentView === 'history' ? 'active' : ''}`}
          onClick={() => setCurrentView('history')}
        >
          {tr.historyLabel}
        </button>
        <button
          className={`sidebar-nav-item ${currentView === 'settings' ? 'active' : ''}`}
          onClick={() => setCurrentView('settings')}
        >
          {tr.settings}
        </button>
      </nav>

      <div className="sidebar-resize-handle" onMouseDown={handleMouseDown} />

      {ctxMenu && (
        <div className="ctx-menu" style={{ left: ctxMenu.x, top: ctxMenu.y }}>
          <button
            className="ctx-menu-item danger"
            onClick={() => {
              setConfirmTarget(ctxMenu.target)
              setCtxMenu(null)
            }}
          >
            🗑 {ctxMenu.target.kind === 'session' ? tr.deleteSessionLabel : tr.deleteProjectLabel}
          </button>
        </div>
      )}

      {confirmTarget && (
        <div className="modal-overlay" onClick={() => setConfirmTarget(null)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{tr.deleteConfirmTitle}</div>
            <div className="modal-message">
              {confirmTarget.kind === 'session'
                ? t(tr, 'deleteSessionConfirmMsg', { name: confirmTarget.label })
                : t(tr, 'deleteProjectConfirmMsg', { name: confirmTarget.label, count: confirmTarget.count })}
            </div>
            <div className="modal-actions">
              <button className="modal-btn" onClick={() => setConfirmTarget(null)}>
                {tr.cancelLabel}
              </button>
              <button className="modal-btn modal-btn-danger" onClick={() => executeDelete(confirmTarget)}>
                {tr.deleteLabel}
              </button>
            </div>
          </div>
        </div>
      )}
    </aside>
  )
}
