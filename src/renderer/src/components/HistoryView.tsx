import React, { useState, useEffect, useMemo, useCallback } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import type { VibeSession } from '../types'

type TimeFilter = '30d' | '90d' | '180d' | 'all'
type SearchMode = 'title' | 'content'

const TIME_FILTER_DAYS: Record<string, number> = { '30d': 30, '90d': 90, '180d': 180 }

function formatDuration(secs: number): string {
  if (secs < 60) return Math.round(secs) + 's'
  if (secs < 3600) return Math.round(secs / 60) + 'm'
  return (secs / 3600).toFixed(1) + 'h'
}

export default function HistoryView(): React.JSX.Element {
  const { config, unhideSessions, unhideProject, reactivateHistorySession } = useStore()
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

  const isSessionHidden = useCallback((s: VibeSession): boolean => {
    const wd = s.environment?.workingDirectory ?? ''
    return hiddenSessions.includes(s.sessionId) || (wd !== '' && hiddenProjects.includes(wd))
  }, [hiddenSessions, hiddenProjects])

  // Distinct project folders for the per-project filter; hidden projects
  // are only listed when "show hidden" is checked.
  const projectList = useMemo(() => {
    const wds = new Set<string>()
    for (const s of sessions) {
      const wd = s.environment?.workingDirectory ?? ''
      if (!wd) continue
      if (!showHidden && hiddenProjects.includes(wd)) continue
      wds.add(wd)
    }
    return [...wds].sort()
  }, [sessions, showHidden, hiddenProjects])

  useEffect(() => {
    if (projectFilter !== '' && !projectList.includes(projectFilter)) setProjectFilter('')
  }, [projectList, projectFilter])

  const filtered = useMemo(() => {
    const days = timeFilter === 'all' ? 0 : TIME_FILTER_DAYS[timeFilter]
    const cutoff = days > 0 ? Date.now() - days * 86400000 : 0
    const q = query.trim().toLowerCase()
    return sessions.filter((s) => {
      if (cutoff && new Date(s.startTime).getTime() < cutoff) return false
      if (!showHidden && isSessionHidden(s)) return false
      if (projectFilter !== '' && (s.environment?.workingDirectory ?? '') !== projectFilter) {
        return false
      }
      if (q) {
        if ((s.title ?? '').toLowerCase().includes(q)) return true
        return searchMode === 'content' && contentMatches[s.sessionId] !== undefined
      }
      return true
    })
  }, [sessions, timeFilter, showHidden, query, searchMode, contentMatches, isSessionHidden, projectFilter])

  const restoreProject = (wd: string): void => {
    const keys = sessions
      .filter((s) => (s.environment?.workingDirectory ?? '') === wd)
      .map((s) => s.sessionId)
    if (keys.length > 0) unhideSessions(keys)
    unhideProject(wd)
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
                {wd.split(/[/\\]/).filter(Boolean).pop() ?? wd}
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
            {filtered.map((s) => {
              const start = new Date(s.startTime)
              const end = new Date(s.endTime)
              const durationSecs = Math.max(0, (end.getTime() - start.getTime()) / 1000)
              const wd = s.environment?.workingDirectory ?? ''
              const project = wd ? wd.split(/[/\\]/).pop() || '—' : '—'
              const model = s.config?.activeModel ?? 'unknown'
              const hidden = isSessionHidden(s)
              const snippet = searchMode === 'content' && query.trim().length >= 2 ? contentMatches[s.sessionId] : undefined
              return (
                <div key={s.sessionId} className={'session-row session-row-clickable history-row' + (hidden ? ' history-row-hidden' : '')}>
                  <div className="session-row-main" onClick={() => reactivateHistorySession(s)} title={tr.reactivateHint}>
                    <span className="session-row-title">
                      {s.title ?? tr.untitledSession}
                      {hidden ? ' (' + tr.hiddenBadge + ')' : ''}
                    </span>
                    <div className="session-row-meta">
                      <span>{start.toLocaleDateString()}</span>
                      <span>{formatDuration(durationSecs)}</span>
                      <span>{tr.messagesLabel} {s.totalMessages}</span>
                      <span>{tr.costLabel} ${(s.stats.sessionCost ?? 0).toFixed(2)}</span>
                    </div>
                    {snippet && <div className="history-snippet">…{snippet}…</div>}
                  </div>
                  <div className="session-row-tags">
                    <span className="tag primary">{project}</span>
                    <span className="tag accent">{model}</span>
                  </div>
                  <div className="history-row-actions">
                    {hidden && (
                      <button
                        className="history-restore-btn"
                        onClick={(e) => {
                          e.stopPropagation()
                          unhideSessions([s.sessionId])
                          if (wd && hiddenProjects.includes(wd)) unhideProject(wd)
                        }}
                      >
                        {tr.restoreLabel}
                      </button>
                    )}
                    <button
                      className="history-reactivate-btn"
                      onClick={(e) => {
                        e.stopPropagation()
                        reactivateHistorySession(s)
                      }}
                    >
                      {tr.reactivateLabel}
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}
