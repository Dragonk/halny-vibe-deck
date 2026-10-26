import React, { useEffect, useState } from 'react'
import { useTranslation } from '../i18n'
import type { AppConfig } from '../types'

/**
 * v0.13.14: unified folder picker for creating a new project - the same
 * dialog runs in the desktop app and in the web client, and browses either
 * the serve host (remoteId = null) or a remote SSH connection. The picked
 * folder (optionally created under the browsed directory) is handed back
 * to the caller, which decides how the session starts.
 */

interface Props {
  config: AppConfig
  remoteId: string | null
  remoteName?: string
  onCreated: (path: string) => void
  onClose: () => void
}

const joinPath = (dir: string, name: string): string =>
  dir === '/' ? '/' + name : dir.replace(/\/+$/, '') + '/' + name

const parentOf = (dir: string): string => {
  if (dir === '/') return '/'
  const cut = dir.replace(/\/+$/, '').lastIndexOf('/')
  return cut <= 0 ? '/' : dir.slice(0, cut)
}

interface DirEntry {
  name: string
  isDir: boolean
}

export default function NewProjectModal({ config, remoteId, remoteName, onCreated, onClose }: Props): React.JSX.Element {
  const tr = useTranslation(config.language)
  const [cwd, setCwd] = useState('/')
  const [dirs, setDirs] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    void window.api.fs
      .listDir(cwd, remoteId ?? undefined)
      .then((entries) => {
        if (cancelled) return
        setDirs(
          ((entries ?? []) as DirEntry[])
            .filter((e) => e.isDir)
            .map((e) => e.name)
            .sort((a, b) => a.localeCompare(b)),
        )
      })
      .catch(() => {
        if (!cancelled) setDirs([])
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [cwd, remoteId])

  const handleCreate = async (): Promise<void> => {
    const n = name.trim()
    if (n.includes('/') || n.includes('\\')) {
      setError(tr.newProjectErrorName)
      return
    }
    const target = n ? joinPath(cwd, n) : cwd
    if (n) {
      setCreating(true)
      try {
        const res = (await window.api.fs.mkdir(target, remoteId ?? undefined)) as { error: string | null }
        if (res?.error) {
          setError(res.error)
          return
        }
      } catch (err) {
        setError(String((err as Error)?.message ?? err))
        return
      } finally {
        setCreating(false)
      }
    }
    onCreated(target)
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-box" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">
          {tr.newProjectLabel}
          {remoteName ? <span className="np-host"> · {remoteName}</span> : null}
        </div>
        <div className="np-cwd">
          <button
            className="np-up"
            title={tr.newProjectUp}
            disabled={cwd === '/'}
            onClick={() => setCwd(parentOf(cwd))}
          >
            ↑
          </button>
          <span className="np-cwd-path" title={cwd}>
            {cwd}
          </span>
        </div>
        <div className="np-list">
          {loading ? (
            <div className="np-loading">…</div>
          ) : dirs.length === 0 ? (
            <div className="np-loading">{tr.newProjectEmpty}</div>
          ) : (
            dirs.map((d) => (
              <button key={d} className="np-item" onClick={() => setCwd(joinPath(cwd, d))}>
                <span className="np-item-icon">📁</span>
                {d}
              </button>
            ))
          )}
        </div>
        <input
          className="nsd-new-project-input"
          placeholder={tr.newProjectNamePlaceholder}
          value={name}
          autoFocus
          onChange={(e) => {
            setName(e.target.value)
            setError(null)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void handleCreate()
          }}
        />
        {error && <div className="nsd-new-project-error">{error}</div>}
        <div className="modal-actions">
          <button className="modal-btn" onClick={onClose}>
            {tr.cancelLabel}
          </button>
          <button className="modal-btn nsd-new-project-create" onClick={() => void handleCreate()} disabled={creating}>
            {creating ? '…' : tr.newProjectCreate}
          </button>
        </div>
      </div>
    </div>
  )
}
