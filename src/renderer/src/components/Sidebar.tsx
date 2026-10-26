import React, { useState, useRef, useCallback, useEffect } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import NewProjectModal from './NewProjectModal'
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
    historyLoadedSessions,
    config,
    loadAcpSession,
    collapsedProjects,
    toggleProjectCollapse,
    hideSession,
    hideProject,
    remoteStatuses,
    remoteLoading,
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
  // v0.13.14: unified "New project…" picker (desktop + web). remoteId = null
  // browses the local/serve host, a connection id browses that SSH host.
  const [npTarget, setNpTarget] = useState<{ remoteId: string | null; remoteName?: string } | null>(null)
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

  // v1.0.2: pointer events (same fix as the dock handle) so sidebar resizing
  // also works on touchscreens and can never get stuck without a mouseup.
  const handlePointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault()
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      /* capture is best-effort */
    }
    setIsResizing(true)
  }, [])

  useEffect(() => {
    if (!isResizing) return

    const handlePointerMove = (e: PointerEvent): void => {
      const newWidth = Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, e.clientX))
      setSidebarWidth(newWidth)
    }

    const handlePointerUp = (): void => {
      setIsResizing(false)
    }

    document.addEventListener('pointermove', handlePointerMove)
    document.addEventListener('pointerup', handlePointerUp)
    document.addEventListener('pointercancel', handlePointerUp)

    return () => {
      document.removeEventListener('pointermove', handlePointerMove)
      document.removeEventListener('pointerup', handlePointerUp)
      document.removeEventListener('pointercancel', handlePointerUp)
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

  // v0.13.14: unified picker opens for the local/serve host.
  const openNewProjectPicker = (): void => {
    setNpTarget({ remoteId: null })
  }

  const handleSelectSession = async (id: string): Promise<void> => {
    setCurrentSessionId(id)
    setPendingNewSession(false)
    setCurrentView('chat')
    setMobileSidebarOpen(false)

    const session = sessions.find((s) => s.id === id)
    if (!session?.acpSessionId) return
    if (liveSessions.includes(id)) {
      // v0.11.1: chips (values AND option lists) follow the session being
      // viewed - restore exactly what the ACP reported for a live session.
      void useStore.getState().restoreSessionChips(id)
      // v0.9.0: only LIVE sessions resume over ACP (loadAcpSession ensures
      // the right worker is alive). A live session without messages is a
      // freshly primed draft - a real load fills its history.
      if (session.messages.length === 0 && !session.loading) {
        await loadAcpSession(id)
      } else {
        useStore.setState({ activeAcpSessionId: session.acpSessionId })
      }
      return
    }
    // v0.9.0 lazy resume: clicking a session only reads its history from
    // disk (no vibe-acp start). ACP wakes up when the user consciously
    // sends a message in this session.
    // v1.0.11: the old `messages.length === 0` guard starved clients that
    // joined a RUNNING turn: streaming events had already delivered the
    // tail, so history was never fetched and the phone showed only the
    // new messages. Load from disk until the history IS loaded (the flag
    // is per client and set by applySessionHistory) - the file preview is
    // a ~20 ms read, idempotent, and the disk state wins.
    if (
      !historyLoadedSessions.includes(id) &&
      !session.loading &&
      !busySessions.includes(id)
    ) {
      await useStore.getState().previewSession(id)
    }
    // v0.12: rebuild the chip option lists AFTER the preview so they use
    // the session-file config - never the lists of the previous session.
    void useStore.getState().restoreSessionChips(id)
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
      {/* v1.0.9: no ✕ on the mobile sessions panel - the user closes it
          with the back gesture / swipe, or by picking a session. */}

      <div className="sidebar-sessions-header">
        <span className="sidebar-sessions-label">{tr.sessions}</span>
        {/* v1.0.7: the bottom duplicate of this button was removed - the
            compact header button (styled like the remote groups' 🆕) is the
            single entry point for creating a project locally. */}
        <button
          className="pgh-plus sidebar-np-btn"
          onClick={openNewProjectPicker}
          title={tr.newProjectLabel}
        >
          🆕
        </button>
      </div>

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
              {!collapsed && (
                <>
                  {/* v1.0.8: phones cannot reliably hit the tiny "+" in
                      the group header - a full-size row-sized button sits
                      between the project header and its sessions
                      (mobile-only, CSS-gated). */}
                  <button
                    className="project-mobile-new"
                    onClick={() => startLocalDraft(group.fullPath || undefined)}
                  >
                    ＋ {tr.newSessionHere}
                  </button>
                  {group.sessions.map(renderSession)}
                </>
              )}
            </div>
          )
        })}
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
                {remoteLoading[conn.id] ? (
                  // v1.0.9: the SSH list is being fetched (SSH handshake +
                  // ACP session/list can take a while) - say so instead of
                  // looking like "no sessions".
                  <span className="remote-loading" title={tr.remoteLoading}>
                    ⟳
                  </span>
                ) : (
                  <span
                    className={`remote-dot ${st?.ok ? 'ok' : 'down'}`}
                    title={st?.ok ? tr.remoteConnUp : tr.remoteConnDown}
                  />
                )}
                <span
                  className="pgh-plus"
                  title={tr.newProjectLabel + ' · ' + conn.name}
                  onClick={(e) => {
                    e.stopPropagation()
                    setNpTarget({ remoteId: conn.id, remoteName: conn.name })
                  }}
                >
                  🆕
                </span>
              </button>
              {!collapsed && (
                <>
                  {/* v1.0.8: mobile-sized button for the remote group too
                      (same tap problem as local projects). */}
                  <button
                    className="project-mobile-new"
                    onClick={() => setNpTarget({ remoteId: conn.id, remoteName: conn.name })}
                  >
                    🆕 {tr.newProjectLabel} · {conn.name}
                  </button>
                  {(() => {
                    // v0.13.1: unified per-remote folder list - configured
                    // locations plus session cwds, deduplicated. A configured
                    // folder without sessions keeps the old location-item
                    // behaviour: clicking it starts a new session there.
                    const normWd = (p: string) => (p.length > 1 && p.endsWith('/') ? p.slice(0, -1) : p)
                    const wdGroups = new Map<string, typeof connSessions>()
                    for (const s of connSessions) {
                      const wd = s.workingDirectory || ''
                      const list = wdGroups.get(wd)
                      if (list) list.push(s)
                      else wdGroups.set(wd, [s])
                    }
                    const groupFor = (target: string): typeof connSessions => {
                      const nt = normWd(target)
                      for (const [gwd, glist] of wdGroups) {
                        if (normWd(gwd) === nt) return glist
                      }
                      return [] as typeof connSessions
                    }
                    const seen = new Set<string>()
                    const folders: Array<{ wd: string; list: typeof connSessions }> = []
                    for (const loc of conn.locations) {
                      const nt = normWd(loc)
                      if (seen.has(nt)) continue
                      seen.add(nt)
                      folders.push({ wd: loc, list: groupFor(loc) })
                    }
                    for (const [gwd, glist] of wdGroups) {
                      const nt = normWd(gwd)
                      if (seen.has(nt)) continue
                      seen.add(nt)
                      folders.push({ wd: gwd, list: glist })
                    }
                    return folders.map(({ wd, list }) => {
                      const hasSessions = list.length > 0
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
                            onClick={() => {
                              if (hasSessions) toggleProjectCollapse(grpKey)
                              else {
                                setMobileSidebarOpen(false)
                                void startRemoteSession(conn.id, wd)
                              }
                            }}
                            title={wd || tr.noProject}
                          >
                            {hasSessions ? (
                              <span className={'project-chevron ' + (grpCollapsed ? '' : 'open')}>▸</span>
                            ) : (
                              <span className="remote-loc-icon">📁</span>
                            )}
                            <span className="project-name">{label}</span>
                            {hasSessions ? <span className="project-count">{list.length}</span> : null}
                            <span
                              className="pgh-plus"
                              title={tr.newSessionHere}
                              onClick={(e) => {
                                e.stopPropagation()
                                setMobileSidebarOpen(false)
                                void startRemoteSession(conn.id, wd)
                              }}
                            >
                              ＋
                            </span>
                          </button>
                          {hasSessions && !grpCollapsed && list.map((s) => renderSession(s))}
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
          onClick={() => {
            setMobileSidebarOpen(false)
            // v1.0.9: the back gesture returns to the chat from Stats.
            useStore.getState().pushBack(() => useStore.getState().setCurrentView('chat'))
            setCurrentView('stats')
          }}
        >
          {tr.statistics}
        </button>
        <button
          className={`sidebar-nav-item ${currentView === 'history' ? 'active' : ''}`}
          onClick={() => {
            setMobileSidebarOpen(false)
            // v1.0.9: the back gesture returns to the chat from History.
            useStore.getState().pushBack(() => useStore.getState().setCurrentView('chat'))
            setCurrentView('history')
          }}
        >
          {tr.historyLabel}
        </button>
        <button
          className={`sidebar-nav-item ${currentView === 'settings' ? 'active' : ''}`}
          onClick={() => {
            setMobileSidebarOpen(false)
            // v1.0.9: the back gesture returns to the chat from Settings.
            useStore.getState().pushBack(() => useStore.getState().setCurrentView('chat'))
            setCurrentView('settings')
          }}
        >
          {tr.settings}
        </button>
      </nav>

      <div className="sidebar-resize-handle" onPointerDown={handlePointerDown} />

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

      {npTarget && (
        <NewProjectModal
          config={config}
          remoteId={npTarget.remoteId}
          remoteName={npTarget.remoteName}
          onClose={() => setNpTarget(null)}
          onCreated={(p) => {
            const rid = npTarget.remoteId
            setNpTarget(null)
            if (rid) void startRemoteSession(rid, p)
            else startLocalDraft(p)
          }}
        />
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
