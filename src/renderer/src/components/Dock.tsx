import React, { useCallback, useEffect, useRef, useState } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import FileTree from './FileTree'
import GitPanel from './GitPanel'
import FilePreview from './FilePreview'

const MIN_DOCK_WIDTH = 260
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

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      setIsResizing(true)
      startXRef.current = e.clientX
      startWRef.current = dockWidth
    },
    [dockWidth],
  )

  useEffect(() => {
    if (!isResizing) return

    const handleMouseMove = (e: MouseEvent): void => {
      const delta = startXRef.current - e.clientX
      const newWidth = Math.min(currentMaxDockWidth(), Math.max(MIN_DOCK_WIDTH, startWRef.current + delta))
      setDockWidth(newWidth)
    }

    const handleMouseUp = (): void => {
      setIsResizing(false)
      // Persist width once the drag ends
      const store = useStore.getState()
      window.api.config.save({ ...store.config, dockWidth: store.dockWidth }).catch(console.error)
    }

    document.addEventListener('mousemove', handleMouseMove)
    document.addEventListener('mouseup', handleMouseUp)
    return () => {
      document.removeEventListener('mousemove', handleMouseMove)
      document.removeEventListener('mouseup', handleMouseUp)
    }
  }, [isResizing, setDockWidth])

  // v0.13.8: keep the dock inside the window when the window itself shrinks
  // (or a width persisted on a bigger monitor is restored).
  useEffect(() => {
    const clampDockWidth = (): void => {
      const w = useStore.getState().dockWidth
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
      style={{ width: dockWidth }}
    >
      <div className="dock-resize-handle" onMouseDown={handleMouseDown} />
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
