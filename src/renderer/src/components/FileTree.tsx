import React, { useEffect, useMemo, useState } from 'react'
import { useStore, fileTreeKey } from '../store'
import { useTranslation } from '../i18n'
import { isWebMode, downloadFile } from '../webBridge'

const BS = String.fromCharCode(92)

const SKIP_DIRS = new Set([
  'node_modules',
  '.git',
  '.venv',
  'venv',
  '__pycache__',
  'dist',
  'build',
  'out',
  'target',
  '.next',
  '.cache',
  '.turbo',
  '.idea',
  '.vscode',
])

interface DirEntry {
  name: string
  isDir: boolean
}

interface Props {
  cwd: string
  /** v0.7.0: remote connection id (null = local filesystem) */
  remoteId?: string | null
}

interface CtxTarget {
  path: string
  name: string
  isDir: boolean
}

// Normalize a directory path so the load key and the render key always
// match (Windows paths arrive with backslashes; keys must be stable).
function normalizeDirPath(dir: string): string {
  return dir.split(BS).join('/')
}

const parentDir = (p: string): string => p.replace(/[\\/][^\\/]*$/, '') || '/'

// The expansion state and directory entries live in the app store, so they
// survive switching dock tabs (e.g. opening a file preview and coming back).
export default function FileTree({ cwd, remoteId }: Props): React.JSX.Element {
  const { openFilePreview, openFileEditor, config, fileTreeExpanded, fileTreeEntries, loadFileTreeDir, toggleFileTreeDir, setAcpError } = useStore()
  const tr = useTranslation(config.language)
  const rid = remoteId ?? null

  // v1.0.9: context menu (right-click on desktop, long-press on touch) -
  // preview / edit / download / open externally / rename / delete.
  const [ctx, setCtx] = useState<{ x: number; y: number; target: CtxTarget } | null>(null)
  // v1.0.26: drag & drop upload - the hovered drop target + a busy flag
  const [dropTarget, setDropTarget] = useState<string | null>(null)
  const [uploading, setUploading] = useState(false)
  // v1.0.26: pending folder auto-expand timers (one per hovered folder)
  const holdTimers = React.useRef<Map<string, number>>(new Map())
  const [renaming, setRenaming] = useState<CtxTarget | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [confirmDel, setConfirmDel] = useState<CtxTarget | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (cwd) loadFileTreeDir(cwd, rid)
  }, [cwd, rid, loadFileTreeDir])

  const rootName = useMemo(
    () => (cwd ? cwd.split(/[/\\]/).filter(Boolean).pop() ?? cwd : ''),
    [cwd],
  )

  const refreshParent = (path: string): void => {
    void loadFileTreeDir(normalizeDirPath(parentDir(path)), rid)
    void loadFileTreeDir(normalizeDirPath(cwd), rid)
  }

  const doRename = async (): Promise<void> => {
    if (!renaming) return
    const clean = renameValue.trim()
    if (!clean || clean === renaming.name) {
      setRenaming(null)
      return
    }
    setBusy(true)
    const res = (await window.api.fs.rename(renaming.path, clean, rid ?? undefined)) as { error: string | null }
    setBusy(false)
    setRenaming(null)
    if (res?.error) {
      console.error('Rename failed:', res.error)
      return
    }
    refreshParent(renaming.path)
  }

  const doDelete = async (): Promise<void> => {
    if (!confirmDel) return
    setBusy(true)
    const res = (await window.api.fs.delete(confirmDel.path, rid ?? undefined)) as { error: string | null }
    setBusy(false)
    setConfirmDel(null)
    if (res?.error) {
      console.error('Delete failed:', res.error)
      return
    }
    refreshParent(confirmDel.path)
  }

  // v1.0.9: download always works - desktop uses a save dialog (remote
  // files are pulled over ssh), the web client uses /api/download for the
  // serve host and a client-side blob for remotes.
  const doDownload = async (t: CtxTarget): Promise<void> => {
    if (isWebMode()) {
      if (rid) {
        try {
          const b = (await window.api.fs.readBinary(t.path, rid ?? undefined)) as { base64: string; mime: string; error: string | null }
          if (b.error) return
          const url = URL.createObjectURL(new Blob([Uint8Array.from(atob(b.base64), (c) => c.charCodeAt(0))], { type: b.mime || 'application/octet-stream' }))
          const a = document.createElement('a')
          a.href = url
          a.download = t.name
          a.click()
          URL.revokeObjectURL(url)
        } catch (err) {
          console.error('Download failed:', err)
        }
      } else {
        await downloadFile(t.path)
      }
    } else {
      const res = (await window.api.fs.saveAs(t.path, rid ?? undefined)) as { error: string | null }
      if (res?.error) console.error('Download failed:', res.error)
    }
  }

  // Long-press (touch) opens the same menu as right-click.
  const pressTimer = React.useRef<number | null>(null)
  const clearPress = (): void => {
    if (pressTimer.current !== null) {
      window.clearTimeout(pressTimer.current)
      pressTimer.current = null
    }
  }
  const onItemTouchStart = (e: React.TouchEvent, target: CtxTarget): void => {
    const t = e.touches[0]
    if (!t) return
    clearPress()
    pressTimer.current = window.setTimeout(() => {
      pressTimer.current = null
      setCtx({ x: t.clientX, y: t.clientY, target })
    }, 550)
  }
  const onItemTouchEnd = (e: React.TouchEvent): void => {
    // A fired long-press must not ALSO trigger the click that follows.
    if (pressTimer.current === null) e.preventDefault()
    clearPress()
  }

  // Close the menu on any click outside of it
  useEffect(() => {
    if (!ctx) return
    const close = (e: MouseEvent): void => {
      const el = e.target as HTMLElement | null
      if (el && el.closest('.ctx-menu')) return
      setCtx(null)
    }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [ctx])

  if (!cwd) {
    return <div className="dock-empty">{tr.openFolderFirst}</div>
  }

  // v1.0.26: drop files into a directory (local disk or remote via ssh)
  const handleDropUpload = async (dir: string, e: React.DragEvent): Promise<void> => {
    e.preventDefault()
    e.stopPropagation()
    setDropTarget(null)
    const files = Array.from(e.dataTransfer?.files ?? [])
    if (files.length === 0) return
    setUploading(true)
    let failed = 0
    for (const file of files) {
      try {
        const b64 = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader()
          reader.onload = () => resolve(String(reader.result ?? '').split(',')[1] ?? '')
          reader.onerror = () => reject(reader.error)
          reader.readAsDataURL(file)
        })
        const res = (await window.api.fs.writeBinary(
          dir.replace(/\/+$/, '') + '/' + file.name,
          b64,
          rid ?? undefined,
        )) as { error: string | null }
        if (res?.error) failed++
      } catch {
        failed++
      }
    }
    setUploading(false)
    void loadFileTreeDir(dir, rid)
    if (failed > 0) setAcpError(`${failed}/${files.length} plików nie wgrano`)
  }

  const renderEntries = (dir: string, depth: number): React.JSX.Element => {
    const entries = fileTreeEntries[fileTreeKey(dir, rid)] ?? []
    return (
      <>
        {entries
          .filter((e) => !(e.isDir && SKIP_DIRS.has(e.name)))
          .map((entry) => {
            const fullPath = `${dir}/${entry.name}`.split(BS).join('/')
            const isExpanded = fileTreeExpanded.has(fileTreeKey(fullPath, rid))
            const target: CtxTarget = { path: fullPath, name: entry.name, isDir: entry.isDir }
            return (
              <div key={fullPath}>
                <button
                  className={`filetree-item ${dropTarget === fullPath ? 'drop-target' : ''}`}
                  style={{ paddingLeft: 6 + depth * 12 }}
                  onDragOver={(e) => {
                    e.preventDefault()
                    e.stopPropagation()
                    setDropTarget(fullPath)
                    // v1.0.26: hovering over a folder briefly expands it
                    if (entry.isDir && !fileTreeExpanded.has(fileTreeKey(fullPath, rid))) {
                      const t = holdTimers.current.get(fullPath)
                      if (!t) holdTimers.current.set(fullPath, window.setTimeout(() => {
                        holdTimers.current.delete(fullPath)
                        toggleFileTreeDir(fullPath, rid)
                      }, 800))
                    }
                  }}
                  onDragLeave={(e) => {
                    if (e.currentTarget === e.target) setDropTarget(null)
                    const t = holdTimers.current.get(fullPath)
                    if (t) { window.clearTimeout(t); holdTimers.current.delete(fullPath) }
                  }}
                  onDrop={(e) => { e.stopPropagation(); setDropTarget(null); void handleDropUpload(fullPath, e) }}
                  onClick={() => {
                    if (entry.isDir) toggleFileTreeDir(fullPath, rid)
                    else openFilePreview(fullPath, entry.name, rid)
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault()
                    setCtx({ x: e.clientX, y: e.clientY, target })
                  }}
                  onTouchStart={(e) => onItemTouchStart(e, target)}
                  onTouchEnd={onItemTouchEnd}
                  onTouchMove={clearPress}
                  title={fullPath}
                >
                  {entry.isDir ? (
                    <span className={`filetree-chevron ${isExpanded ? 'open' : ''}`}>▸</span>
                  ) : (
                    <span className="filetree-chevron invisible">▸</span>
                  )}
                  <span className={entry.isDir ? 'filetree-dir' : 'filetree-file'}>
                    {entry.name}
                  </span>
                </button>
                {entry.isDir && isExpanded && renderEntries(fullPath, depth + 1)}
              </div>
            )
          })}
        {entries.length === 0 && (
          <div className="filetree-empty" style={{ paddingLeft: 6 + depth * 12 }}>
            {tr.fileTreeEmpty}
          </div>
        )}
      </>
    )
  }

  return (
    <div className="filetree">
      <div
        className={`filetree-root ${dropTarget === 'ROOT' ? 'drop-target' : ''}`}
        title={cwd}
        onDragOver={(e) => { e.preventDefault(); setDropTarget('ROOT') }}
        onDragLeave={(e) => { if (e.currentTarget === e.target) setDropTarget(null) }}
        onDrop={(e) => { e.stopPropagation(); setDropTarget(null); void handleDropUpload(normalizeDirPath(cwd), e) }}
      >
        <span className="filetree-root-name">📁 {rootName}</span>
      </div>
      <div className="filetree-body">{renderEntries(normalizeDirPath(cwd), 0)}</div>
      {uploading && <div className="filetree-uploading">⬆ …</div>}

      {ctx && (
        <div className="ctx-menu" style={{ left: ctx.x, top: ctx.y }}>
          <button
            className="ctx-menu-item"
            onClick={() => {
              if (ctx.target.isDir) toggleFileTreeDir(ctx.target.path, rid)
              else openFilePreview(ctx.target.path, ctx.target.name, rid)
              setCtx(null)
            }}
          >
            👁 {tr.menuPreview}
          </button>
          {!ctx.target.isDir && (
            <button
              className="ctx-menu-item"
              onClick={() => {
                openFileEditor(ctx.target.path, ctx.target.name, rid)
                setCtx(null)
              }}
            >
              ✏️ {tr.menuEdit}
            </button>
          )}
          <button
            className="ctx-menu-item"
            onClick={() => {
              void doDownload(ctx.target)
              setCtx(null)
            }}
          >
            ⬇ {tr.downloadLabel}
          </button>
          {!isWebMode() && !rid && (
            <button
              className="ctx-menu-item"
              onClick={() => {
                void window.api.shell.openPath(ctx.target.path)
                setCtx(null)
              }}
            >
              ↗ {tr.previewOpenExternally}
            </button>
          )}
          <button
            className="ctx-menu-item"
            onClick={() => {
              setRenaming(ctx.target)
              setRenameValue(ctx.target.name)
              setCtx(null)
            }}
          >
            📝 {tr.menuRename}
          </button>
          <button
            className="ctx-menu-item danger"
            onClick={() => {
              setConfirmDel(ctx.target)
              setCtx(null)
            }}
          >
            🗑 {tr.menuDelete}
          </button>
        </div>
      )}

      {renaming && (
        <div className="modal-overlay" onClick={() => setRenaming(null)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{tr.menuRename}</div>
            <input
              className="nsd-new-project-input"
              value={renameValue}
              autoFocus
              onChange={(e) => setRenameValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void doRename()
                if (e.key === 'Escape') setRenaming(null)
              }}
            />
            <div className="modal-actions">
              <button className="modal-btn" onClick={() => setRenaming(null)}>
                {tr.cancelLabel}
              </button>
              <button className="modal-btn nsd-new-project-create" onClick={() => void doRename()} disabled={busy}>
                {busy ? '…' : tr.saveLabel}
              </button>
            </div>
          </div>
        </div>
      )}

      {confirmDel && (
        <div className="modal-overlay" onClick={() => setConfirmDel(null)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{tr.menuDelete}</div>
            <p className="confirm-text">{tr.deleteConfirmText.replace('{name}', confirmDel.name)}</p>
            <div className="modal-actions">
              <button className="modal-btn" onClick={() => setConfirmDel(null)}>
                {tr.cancelLabel}
              </button>
              <button className="modal-btn danger" onClick={() => void doDelete()} disabled={busy}>
                {busy ? '…' : tr.menuDelete}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
