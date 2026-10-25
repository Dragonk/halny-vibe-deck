import React, { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import type { VibeSession } from '../types'

type TimeFilter = '30d' | '90d' | '180d' | 'all'
type SearchMode = 'title' | 'content'

const TIME_FILTER_DAYS: Record<string, number> = { '30d': 30, '90d': 90, '180d': 180 }
// Rows render lazily: PAGE_SIZE at a time, grown when the sentinel scrolls in.
const PAGE_SIZE = 100

function formatDuration(secs: number): string {
  if (secs < 60) return Math.round(secs) + 's'
  if (secs < 3600) return Math.round(secs / 60) + 'm'
  return (secs / 3600).toFixed(1) + 'h'
}

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p
}

interface LocalRow {
  kind: 'local'
  id: string
  t: number
  hidden: boolean
  wd: string
  session: VibeSession
}

interface RemoteRow {
  kind: 'remote'
  id: string
  t: number
  hidden: boolean
  wd: string
  remoteId: string
  connName: string
  sessionId: string
  title: string
}

type HistoryRow = LocalRow | RemoteRow

export default function HistoryView(): React.JSX.Element {
  const { config, unhideSessions, unhideProject, reactivateHistorySession, loadRemoteSession } = useStore()
  const tr = useTranslation(config.language)
  const [sessions, setSessions] = useState<VibeSession[]>([])
  const [loading, setLoading] = useState(true)
  const [query, setQuery] = useState('')
  const [searchMode, setSearchMode] = useState<SearchMode>('title')
  const [timeFilter, setTimeFilter] = useState<TimeFilter>('all')
  const [showHidden, setShowHidden] = useState(false)
  const [projectFilter, setProjectFilter] = useState('')
  const [contentMatches, setContentMatches] = useState<Record<string, string>>({})
  const [searching, setSearching] = useState(false)
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE)
  const storeSessions = useStore((s) => s.sessions)
  const sentinelRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    let cancelled = false
    window.api.stats.vibeSessions().then((res) => {
      if (cancelled) return
      const list = res as VibeSession[]
      list.sort((a, b) => new Date(b.startTime).getTime() - new Date(a.startTime).getTime())
      setSessions(list)
      setLoading(false)
    }).catch(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    const q = query.trim()
    if (searchMode !== 'content' || q.length < 2) {
      setContentMatches({})
      return
    }
    let cancelled = false
    const h = window.setTimeout(() => {
      setSearching(true)
      window.api.stats.searchSessionContent(q).then((res) => {
        if (cancelled) return
        const map: Record<string, string> = {}
        for (const m of res) map[m.sessionId] = m.snippet
        setContentMatches(map)
        setSearching(false)
      }).catch(() => { if (!cancelled) setSearching(false) })
    }, 400)
    return () => { cancelled = true; window.clearTimeout(h) }
  }, [query, searchMode])

  const hiddenSessions = config.hiddenSessions ?? []
  const hiddenProjects = config.hiddenProjects ?? []
  const remoteConns = config.remoteConnections ?? []

  const isHidden = useCallback((id: string, wd: string): boolean => {
    return hiddenSessions.includes(id) || (wd !== '' && hiddenProjects.includes(wd))
  }, [hiddenSessions, hiddenProjects])

  // Remote (SSH) sessions never touch the local disk, so stats.vibeSessions()
  // cannot see them. The app imports stubs for every enabled connection -
  // reuse them here so the history covers remote work too.
  const remoteRows = useMemo<RemoteRow[]>(() => {
    const rows: RemoteRow[] = []
    const seen = new Set<string>()
    for (const s of storeSessions) {
      if (!s.remoteId || !s.acpSessionId) continue
      const key = s.remoteId + ':' + s.acpSessionId
      if (seen.has(key)) continue
      seen.add(key)
      const wd = s.workingDirectory ?? ''
      rows.push({
        kind: 'remote',
        id: key,
        t: s.updatedAt ?? s.lastActive ?? s.createdAt ?? 0,
        hidden: isHidden(s.acpSessionId, wd),
        wd,
        remoteId: s.remoteId,
        connName: remoteConns.find((c) => c.id === s.remoteId)?.name ?? s.remoteId,
        sessionId: s.acpSessionId,
        title: s.name,
      })
    }
    return rows
  }, [storeSessions, remoteConns, isHidden])

  const projectList = useMemo(() => {
    const wds = new Set<string>()
    for (const s of sessions) {
      const wd = s.environment?.workingDirectory ?? ''
      if (!wd) continue
      if (!showHidden && hiddenProjects.includes(wd)) continue
      wds.add(wd)
    }
    for (const r of remoteRows) {
      if (!r.wd) continue
      if (!showHidden && hiddenProjects.includes(r.wd)) continue
      wds.add(r.wd)
    }
    return [...wds].sort()
  }, [sessions, remoteRows, showHidden, hiddenProjects])

  useEffect(() => {
    if (projectFilter !== '' && !projectList.includes(projectFilter)) setProjectFilter('')
  }, [projectList, projectFilter])

  const filtered = useMemo<HistoryRow[]>(() => {
    const days = timeFilter === 'all' ? 0 : TIME_FILTER_DAYS[timeFilter]
    const cutoff = days > 0 ? Date.now() - days * 86400000 : 0
    const q = query.trim().toLowerCase()
    const rows: HistoryRow[] = []
    for (const s of sessions) {
      const wd = s.environment?.workingDirectory ?? ''
      const t0 = new Date(s.startTime).getTime()
      if (cutoff && t0 < cutoff) continue
      const hidden = isHidden(s.sessionId, wd)
      if (!showHidden && hidden) continue
      if (projectFilter !== '' && wd !== projectFilter) continue
      if (q) {
        const inTitle = (s.title ?? '').toLowerCase().includes(q)
        const inContent = searchMode === 'content' && contentMatches[s.sessionId] !== undefined
        if (!inTitle && !inContent) continue
      }
      rows.push({ kind: 'local', id: s.sessionId, t: t0, hidden, wd, session: s })
    }
    for (const r of remoteRows) {
      if (cutoff && r.t < cutoff) continue
      if (!showHidden && r.hidden) continue
      if (projectFilter !== '' && r.wd !== projectFilter) continue
      if (q && !r.title.toLowerCase().includes(q)) continue
      rows.push(r)
    }
    rows.sort((a, b) => b.t - a.t)
    return rows
  }, [sessions, remoteRows, timeFilter, showHidden, query, searchMode, contentMatches, isHidden, projectFilter])

  // Dynamic loading: grow the rendered window when the sentinel row at the
  // bottom of the list scrolls into view.
  useEffect(() => {
    const el = sentinelRef.current
    if (!el || typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) setVisibleCount((v) => v + PAGE_SIZE)
        }
      },
      { root: null, rootMargin: '240px' },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [loading, filtered.length > 0])

  useEffect(() => {
    setVisibleCount(PAGE_SIZE)
  }, [query, searchMode, timeFilter, showHidden, projectFilter])

  const visible = filtered.slice(0, visibleCount)

  const restoreProject = (wd: string): void => {
    const keys = sessions
      .filter((s) => (s.environment?.workingDirectory ?? '') === wd)
      .map((s) => s.sessionId)
    if (keys.length > 0) unhideSessions(keys)
    unhideProject(wd)
  }

  const activate = (row: HistoryRow): void => {
    if (row.kind === 'local') {
      reactivateHistorySession(row.session)
      return
    }
    void loadRemoteSession(row.remoteId, {
      sessionId: row.sessionId,
      title: row.title,
      cwd: row.wd,
      updatedAt: row.t > 0 ? new Date(row.t).toISOString() : '',
    })
  }

  const restoreRow = (row: HistoryRow): void => {
    unhideSessions([row.kind === 'local' ? row.session.sessionId : row.sessionId])
    if (row.wd !== '' && hiddenProjects.includes(row.wd)) unhideProject(row.wd)
  }

  return (
    <div className="history-view">
      <div className="history-header">
        <h2 className="settings-title">{tr.historyLabel}</h2>
        <div className="history-controls">
          <div className="scope-switch">
            <button className={'scope-pill' + (timeFilter === '30d' ? ' active' : '')} onClick={() => setTimeFilter('30d')}>{tr.time30d}</button>
            <button className={'scope-pill' + (timeFilter === '90d' ? ' active' : '')} onClick={() => setTimeFilter('90d')}>{tr.time90d}</button>
            <button className={'scope-pill' + (timeFilter === '180d' ? ' active' : '')} onClick={() => setTimeFilter('180d')}>{tr.time180d}</button>
            <button className={'scope-pill' + (timeFilter === 'all' ? ' active' : '')} onClick={() => setTimeFilter('all')}>{tr.timeAll}</button>
          </div>
          <div className="scope-switch">
            <button className={'scope-pill' + (searchMode === 'title' ? ' active' : '')} onClick={() => setSearchMode('title')}>{tr.searchTitleLabel}</button>
            <button className={'scope-pill' + (searchMode === 'content' ? ' active' : '')} onClick={() => setSearchMode('content')}>{tr.searchContentLabel}</button>
          </div>
          <input
            className="field-input history-search"
            type="text"
            value={query}
            placeholder={searchMode === 'content' ? tr.searchContentPlaceholder : tr.searchTitlePlaceholder}
            onChange={(e) => setQuery(e.target.value)}
          />
          {searching && <span className="history-searching">…</span>}
          <select
            className="field-select history-project-select"
            value={projectFilter}
            onChange={(e) => setProjectFilter(e.target.value)}
          >
            <option value="">{tr.allProjects}</option>
            {projectList.map((wd) => (
              <option key={wd} value={wd} title={wd}>
                {baseName(wd)}
              </option>
            ))}
          </select>
          <label className="history-show-hidden">
            <input
              type="checkbox"
              checked={showHidden}
              onChange={(e) => setShowHidden(e.target.checked)}
            />
            {tr.showHiddenLabel}
          </label>
        </div>
      </div>

      {showHidden && hiddenProjects.length > 0 && (
        <div className="history-hidden-projects">
          <div className="section-label">{tr.hiddenProjectsLabel}</div>
          {hiddenProjects.map((wd) => (
            <div key={wd} className="history-hidden-row">
              <span className="history-hidden-path" title={wd}>{wd}</span>
              <button className="history-restore-btn" onClick={() => restoreProject(wd)}>
                {tr.restoreLabel}
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="history-content">
        {loading ? (
          <div className="empty-text">{tr.loading}...</div>
        ) : filtered.length === 0 ? (
          <div className="empty-text">{tr.noSessionsFound}</div>
        ) : (
          <div className="session-rows">
            {visible.map((row) => {
              const start = new Date(row.t)
              const title = row.kind === 'local'
                ? (row.session.title ?? tr.untitledSession)
                : (row.title !== '' ? row.title : tr.untitledSession)
              const project = row.wd !== '' ? baseName(row.wd) : '—'
              return (
                <div key={row.id} className={'session-row session-row-clickable history-row' + (row.hidden ? ' history-row-hidden' : '')}>
                  <div className="session-row-main" onClick={() => activate(row)} title={tr.reactivateHint}>
                    <span className="session-row-title">
                      {title}
                      {row.hidden ? ' (' + tr.hiddenBadge + ')' : ''}
                    </span>
                    <div className="session-row-meta">
                      <span>{start.toLocaleDateString()}</span>
                      {row.kind === 'local' ? (
                        <span>{formatDuration(Math.max(0, (new Date(row.session.endTime).getTime() - row.t) / 1000))}</span>
                      ) : (
                        <span className="history-remote-marker">{tr.remoteTab} · {row.connName}</span>
                      )}
                      {row.kind === 'local' && (
                        <>
                          <span>{tr.messagesLabel} {row.session.totalMessages}</span>
                          <span>{tr.costLabel} ${((row.session.stats?.sessionCost ?? 0)).toFixed(2)}</span>
                        </>
                      )}
                    </div>
                    {row.kind === 'local' && searchMode === 'content' && query.trim().length >= 2 && contentMatches[row.session.sessionId] !== undefined && (
                      <div className="history-snippet">…{contentMatches[row.session.sessionId]}…</div>
                    )}
                  </div>
                  <div className="session-row-tags">
                    <span className="tag primary">{project}</span>
                    <span className="tag accent">{row.kind === 'local' ? (row.session.config?.activeModel ?? 'unknown') : row.connName}</span>
                  </div>
                  <div className="history-row-actions">
                    {row.hidden && (
                      <button
                        className="history-restore-btn"
                        onClick={(e) => { e.stopPropagation(); restoreRow(row) }}
                      >
                        {tr.restoreLabel}
                      </button>
                    )}
                    <button
                      className="history-reactivate-btn"
                      onClick={(e) => { e.stopPropagation(); activate(row) }}
                    >
                      {tr.reactivateLabel}
                    </button>
                  </div>
                </div>
              )
            })}
            <div ref={sentinelRef} className="history-load-more">
              {filtered.length > visible.length
                ? t(tr, 'historyShowingCount', { shown: visible.length, total: filtered.length })
                : ''}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
