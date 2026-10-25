import React, { useCallback, useEffect, useRef, useState } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import FileTree from './FileTree'
import GitPanel from './GitPanel'
import FilePreview from './FilePreview'

const MIN_DOCK_WIDTH = 260
const MAX_DOCK_WIDTH = 720

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
      const newWidth = Math.min(MAX_DOCK_WIDTH, Math.max(MIN_DOCK_WIDTH, startWRef.current + delta))
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

  // The dock only shows in chat view. Keep this check AFTER all hooks so
  // the hook count never changes between renders (React error #310).
  if (!dockOpen || currentView !== 'chat') return null

  return (
    <aside
      className={`dock ${isResizing ? 'dock-resizing' : ''}`}
      style={{ width: dockWidth }}
    >
      <div className="dock-resize-handle" onMouseDown={handleMouseDown} />
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
