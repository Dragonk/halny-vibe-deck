import React, { useEffect, useState, useCallback } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import type { GitStatus, GitCommit } from '../../../preload/index'

interface Props {
  cwd: string
  /** v0.7.0: remote connection id (null = local git) */
  remoteId?: string | null
}

export default function GitPanel({ cwd, remoteId }: Props): React.JSX.Element {
  const { config, openDiffPreview } = useStore()
  const tr = useTranslation(config.language)
  const rid = remoteId ?? null
  const [status, setStatus] = useState<GitStatus | null>(null)
  const [log, setLog] = useState<GitCommit[]>([])
  const [loading, setLoading] = useState(false)
  const [commitMsg, setCommitMsg] = useState('')
  const [gitError, setGitError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    if (!cwd) return
    setLoading(true)
    try {
      const [s, l] = await Promise.all([
        window.api.git.status(cwd, rid ?? undefined),
        window.api.git.log(cwd, 30, rid ?? undefined),
      ])
      setStatus(s)
      setLog(l)
    } catch {
      setStatus(null)
      setLog([])
    } finally {
      setLoading(false)
    }
  }, [cwd, rid])

  useEffect(() => {
    refresh()
    const timer = setInterval(refresh, 30_000)
    return () => clearInterval(timer)
  }, [refresh])

  const stagePaths = useCallback(async (paths: string[]): Promise<void> => {
    const err = await window.api.git.stage(cwd, paths, rid ?? undefined)
    if (err) setGitError(err)
    await refresh()
  }, [cwd, rid, refresh])

  const unstagePaths = useCallback(async (paths: string[]): Promise<void> => {
    const err = await window.api.git.unstage(cwd, paths, rid ?? undefined)
    if (err) setGitError(err)
    await refresh()
  }, [cwd, rid, refresh])

  const stageAll = useCallback(async (): Promise<void> => {
    const unstaged = (status?.files ?? []).filter((f) => !f.staged).map((f) => f.path)
    if (unstaged.length > 0) await stagePaths(unstaged)
  }, [status, stagePaths])

  const handleCommit = useCallback(async (): Promise<void> => {
    if (!commitMsg.trim()) return
    setBusy(true)
    const err = await window.api.git.commit(cwd, commitMsg, rid ?? undefined)
    setBusy(false)
    if (err) { setGitError(err); return }
    setGitError(null)
    setCommitMsg('')
    await refresh()
  }, [cwd, commitMsg, rid, refresh])

  const handlePush = useCallback(async (): Promise<void> => {
    setBusy(true)
    const err = await window.api.git.push(cwd, rid ?? undefined)
    setBusy(false)
    if (err) { setGitError(err); return }
    setGitError(null)
    await refresh()
  }, [cwd, rid, refresh])

  if (!cwd) {
    return <div className="dock-empty">{tr.openFolderFirst}</div>
  }

  if (!status) {
    return (
      <div className="git-panel">
        <div className="git-toolbar">
          <button className="git-refresh-btn" onClick={refresh} title={tr.gitRefresh}>
            ⟳
          </button>
        </div>
        <div className="dock-empty">{tr.gitLoading}</div>
      </div>
    )
  }

  if (!status.repo) {
    return <div className="dock-empty">{tr.notAGitRepo}</div>
  }

  const statusIcon = (f: { index: string; working: string }): string => {
    const code = f.index !== '.' && f.index !== ' ' ? f.index : f.working
    switch (code) {
      case 'A':
        return 'A'
      case 'M':
        return 'M'
      case 'D':
        return 'D'
      case '?':
        return '?'
      case 'R':
        return 'R'
      case 'C':
        return 'C'
      default:
        return code || '·'
    }
  }

  const openDiff = async (filePath: string): Promise<void> => {
    const diff = await window.api.git.diffFile(cwd, filePath, rid ?? undefined)
    openDiffPreview(filePath.split(/[/\\]/).pop() ?? filePath, filePath, diff ?? '', rid)
  }

  return (
    <div className="git-panel">
      <div className="git-toolbar">
        <span className="git-branch" title={status.branch ?? ''}>
          <span className="git-branch-icon">⑂</span>
          {status.branch}
          {(status.ahead > 0 || status.behind > 0) && (
            <span className="git-ahead-behind">
              {status.ahead > 0 && <span> ↑{status.ahead}</span>}
              {status.behind > 0 && <span> ↓{status.behind}</span>}
            </span>
          )}
        </span>
        {status.ahead > 0 && (
          <button className="git-stage-all-btn" onClick={handlePush} disabled={busy} title={tr.pushHint}>
            {tr.pushLabel} ↑{status.ahead}
          </button>
        )}

        <button className="git-refresh-btn" onClick={refresh} title={tr.gitRefresh}>
          {loading ? '…' : '⟳'}
        </button>
      </div>

      {(status.error || gitError) && <div className="git-error">{gitError ?? status.error}</div>}

      <div className="git-section-label">
        {tr.gitChanges} ({status.files.length})
        {status.files.some((f) => !f.staged) && (
          <button className="git-stage-all-btn" onClick={stageAll} title={tr.stageLabel}>
            + {tr.stageLabel}
          </button>
        )}
      </div>
      <div className="git-files">
        {status.files.length === 0 && <div className="git-empty">{tr.gitNoChanges}</div>}
        {status.files.map((f) => (
          <div key={f.path} className="git-file-row">
            <button
              className="git-file"
              onClick={() => openDiff(f.path)}
              title={f.path}
            >
              <span className={`git-status-code git-status-${statusIcon(f)}`}>
                {statusIcon(f)}
              </span>
              <span className="git-file-name">{f.path.split(/[\\]/).pop()}</span>
              <span className="git-file-dir">
                {f.path.includes('/') || f.path.includes('\\')
                  ? f.path.split(/[\\]/).slice(0, -1).join('/')
                  : ''}
              </span>
            </button>
            <button
              className="git-stage-btn"
              title={f.staged ? tr.unstageLabel : tr.stageLabel}
              onClick={() => (f.staged ? unstagePaths([f.path]) : stagePaths([f.path]))}
            >
              {f.staged ? '−' : '+'}
            </button>
          </div>
        ))}
      </div>

      <div className="git-commit-box">
        <input
          className="git-commit-input"
          type="text"
          value={commitMsg}
          placeholder={tr.commitMessagePlaceholder}
          onChange={(e) => { setCommitMsg(e.target.value); setGitError(null) }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void handleCommit()
          }}
        />
        <button
          className="git-commit-btn"
          onClick={handleCommit}
          disabled={busy || !commitMsg.trim()}
        >
          {tr.commitLabel}
        </button>
      </div>

      <div className="git-section-label">{tr.gitHistory}</div>
      <div className="git-log">
        {log.length === 0 && <div className="git-empty">{tr.gitNoCommits}</div>}
        {log.map((c) => (
          <div key={c.hash} className="git-commit" title={`${c.hash}\n${c.author}`}>
            <span className="git-commit-hash">{c.shortHash}</span>
            <span className="git-commit-msg">{c.message}</span>
            <span className="git-commit-meta">
              {c.author.split(' ')[0]} ·{' '}
              {new Date(c.date).toLocaleDateString(undefined, {
                month: '2-digit',
                day: '2-digit',
              })}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
