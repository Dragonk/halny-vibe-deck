import React, { useCallback, useEffect, useRef, useState } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import FileTree from './FileTree'
import GitPanel from './GitPanel'
import FilePreview from './FilePreview'

const MIN_DOCK_WIDTH = 260
// v1.0.2: sane default when no width was ever persisted (fresh installs used
// to render the dock squeezed to its content width, ~200px, with no way to
// see it was resizable).
const DEFAULT_DOCK_WIDTH = 420
// v0.13.8: minimum space kept for the chat column when the dock is dragged
// wide - the dock can now span almost the whole window (the old hard cap
// of 720px wasted most of the screen on ultrawide monitors).
const MIN_CHAT_WIDTH = 360
const DEFAULT_SIDEBAR_WIDTH = 220

// v0.13.8: the sessions sidebar keeps its size while the dock grows - the
// chat column is the flexible one. The cap is computed from the real
// sidebar width at drag time, so a resized sidebar is always respected.
const currentMaxDockWidth = (): number => {
  let sidebarWidth = 0
  try {
    const el = document.querySelector<HTMLElement>('.sidebar')
    if (el && el.offsetWidth > 0) sidebarWidth = el.offsetWidth
  } catch {
    /* fall back to the default width */
  }
  if (!sidebarWidth) sidebarWidth = DEFAULT_SIDEBAR_WIDTH
  return Math.max(MIN_DOCK_WIDTH, window.innerWidth - sidebarWidth - MIN_CHAT_WIDTH)
}

export default function Dock(): React.JSX.Element | null {
  const {
    dockOpen,
    dockWidth,
    setDockWidth,
    dockTab,
    setDockTab,
    previewTabs,
    activePreviewId,
    setActivePreview,
    closePreviewTab,
    currentSessionWorkingDirectory,
    currentView,
    config,
    currentSessionId,
  } = useStore()
  const tr = useTranslation(config.language)
  const [isResizing, setIsResizing] = useState(false)
  const startXRef = useRef(0)
  const startWRef = useRef(0)

  const cwd = currentSessionWorkingDirectory()
  const activeSession = useStore.getState().sessions.find((s) => s.id === currentSessionId) ?? null
  // v0.7.0: dock tabs work for remote sessions too (fs/git over ssh)
  const remoteId = activeSession?.remoteId ?? null
  const activePreview = previewTabs.find((t) => t.id === activePreviewId) ?? null
  const activeTabKind: 'files' | 'git' | 'preview' = activePreview ? 'preview' : dockTab

  // v1.0.2: pointer events instead of mouse events. The mouse-only drag was
  // broken in three real-world cases: touchscreens and the install-from-browser
  // PWA (mouse events never fire at all), a missed mouseup when the drag ended
  // outside the window (which left the dock stuck in resizing mode forever),
  // and move events swallowed while the cursor crossed an iframe or another
  // capture target mid-drag. Pointer capture delivers every move to us until
  // pointerup/pointercancel, for mouse, touch and pen alike.
  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault()
      try {
        e.currentTarget.setPointerCapture(e.pointerId)
      } catch {
        /* capture is best-effort */
      }
      setIsResizing(true)
      startXRef.current = e.clientX
      startWRef.current = dockWidth ?? DEFAULT_DOCK_WIDTH
    },
    [dockWidth],
  )

  useEffect(() => {
    if (!isResizing) return

    const handlePointerMove = (e: PointerEvent): void => {
      const delta = startXRef.current - e.clientX
      const newWidth = Math.min(currentMaxDockWidth(), Math.max(MIN_DOCK_WIDTH, startWRef.current + delta))
      setDockWidth(newWidth)
    }

    const handlePointerUp = (): void => {
      setIsResizing(false)
      // Persist width once the drag ends
      const store = useStore.getState()
      window.api.config.save({ ...store.config, dockWidth: store.dockWidth }).catch(console.error)
    }

    document.addEventListener('pointermove', handlePointerMove)
    document.addEventListener('pointerup', handlePointerUp)
    document.addEventListener('pointercancel', handlePointerUp)
    return () => {
      document.removeEventListener('pointermove', handlePointerMove)
      document.removeEventListener('pointerup', handlePointerUp)
      document.removeEventListener('pointercancel', handlePointerUp)
    }
  }, [isResizing, setDockWidth])

  // v0.13.8: keep the dock inside the window when the window itself shrinks
  // (or a width persisted on a bigger monitor is restored).
  useEffect(() => {
    const clampDockWidth = (): void => {
      const w = useStore.getState().dockWidth ?? DEFAULT_DOCK_WIDTH
      const max = currentMaxDockWidth()
      if (w > max) setDockWidth(max)
    }
    clampDockWidth()
    window.addEventListener('resize', clampDockWidth)
    return () => window.removeEventListener('resize', clampDockWidth)
  }, [setDockWidth])

  // The dock only shows in chat view. Keep this check AFTER all hooks so
  // the hook count never changes between renders (React error #310).
  if (!dockOpen || currentView !== 'chat') return null

  return (
    <aside
      className={`dock ${isResizing ? 'dock-resizing' : ''}`}
      style={{ width: dockWidth ?? DEFAULT_DOCK_WIDTH }}
    >
      <div
        className="dock-resize-handle"
        onPointerDown={handlePointerDown}
      />
      <button
        className="mobile-panel-close"
        title={tr.closePanelLabel}
        onClick={() => useStore.getState().setDockOpen(false)}
      >
        ✕
      </button>
      <div className="dock-tabs">
        <button
          className={`dock-tab ${activeTabKind === 'files' ? 'active' : ''}`}
          onClick={() => {
            setDockTab('files')
            setActivePreview('')
          }}
          title={tr.filesTab}
        >
          {tr.filesTab}
        </button>
        <button
          className={`dock-tab ${activeTabKind === 'git' ? 'active' : ''}`}
          onClick={() => {
            setDockTab('git')
            setActivePreview('')
          }}
          title={tr.gitTab}
        >
          {tr.gitTab}
        </button>
        <span className="dock-tab-sep" />
        {previewTabs.map((tab) => (
          <button
            key={tab.id}
            className={`dock-tab dock-tab-preview ${activePreview?.id === tab.id ? 'active' : ''}`}
            onClick={() => setActivePreview(tab.id)}
            title={tab.path}
          >
            <span className="dock-tab-name">{tab.name}</span>
            <span
              className="dock-tab-close"
              onClick={(e) => {
                e.stopPropagation()
                closePreviewTab(tab.id)
              }}
            >
              ✕
            </span>
          </button>
        ))}
      </div>
      <div className="dock-body">
        {activeTabKind === 'files' && <FileTree cwd={cwd} remoteId={remoteId} />}
        {activeTabKind === 'git' && <GitPanel cwd={cwd} remoteId={remoteId} />}
        {activeTabKind === 'preview' && activePreview && <FilePreview tab={activePreview} />}
      </div>
    </aside>
  )
}
