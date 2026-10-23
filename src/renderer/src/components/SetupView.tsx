import React, { useEffect, useRef, useState } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'

interface SetupStatus {
  vibAcpFound: boolean
  vibAcpPath: string | null
  uvFound: boolean
  uvPath: string | null
}

interface ProgressLine {
  message: string
  type: 'info' | 'error' | 'success'
}

interface Props {
  onComplete: (vibAcpPath: string) => void
  onSkip: () => void
}

export default function SetupView({ onComplete, onSkip }: Props): React.JSX.Element {
  const { config } = useStore()
  const tr = useTranslation(config.language)
  const [phase, setPhase] = useState<'confirm' | 'installing' | 'done' | 'error'>('confirm')
  const [lines, setLines] = useState<ProgressLine[]>([])
  const [uvFound, setUvFound] = useState(false)
  const logRef = useRef<HTMLDivElement>(null)

  // Scroll log to bottom on new lines
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight
    }
  }, [lines])

  // Subscribe to progress events
  useEffect(() => {
    const unsub = window.api.setup.onProgress((ev) => {
      setLines((prev) => [...prev, { message: ev.message, type: ev.type as ProgressLine['type'] }])
    })
    return unsub
  }, [])

  // Check uv status for the confirmation screen
  useEffect(() => {
    window.api.setup.check().then((raw) => setUvFound((raw as SetupStatus).uvFound))
  }, [])

  const handleInstall = async (): Promise<void> => {
    setPhase('installing')
    setLines([])
    const status = (await window.api.setup.install()) as SetupStatus
    if (status.vibAcpFound && status.vibAcpPath) {
      setPhase('done')
    } else {
      setPhase('error')
    }
  }

  return (
    <div className="setup-overlay">
      <div className="setup-card">
        {/* Header */}
        <div className="setup-header">
          <div className="setup-logo">
            <svg width="32" height="32" viewBox="0 0 24 24" fill="var(--accent)">
              <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 14H9V8h2v8zm4 0h-2V8h2v8z" />
            </svg>
          </div>
          <div>
            <div className="setup-title">{tr.appName}</div>
            <div className="setup-subtitle">{tr.configurationRequired}</div>
          </div>
        </div>

        {/* Confirm screen */}
        {phase === 'confirm' && (
          <div className="setup-body">
            <p className="setup-desc" dangerouslySetInnerHTML={{ __html: tr.vibeAcpNotDetected.replace('<strong>vibe-acp</strong>', '<strong>vibe-acp</strong>') }} />
            <div className="setup-steps">
              <div className={`setup-step ${uvFound ? 'step-done' : ''}`}>
                <span className="step-num">{uvFound ? '✓' : '1'}</span>
                <div>
                  <div className="step-name">{tr.uvPackageManager}</div>
                  {uvFound ? (
                    <div className="step-hint ok">{tr.alreadyInstalled}</div>
                  ) : (
                    <div className="step-hint">
                      {tr.downloadedFromAstral}
                    </div>
                  )}
                </div>
              </div>
              <div className="setup-step">
                <span className="step-num">{uvFound ? '1' : '2'}</span>
                <div>
                  <div className="step-name">{tr.mistralVibe}</div>
                  <div className="step-hint">
                    <code>{tr.uvToolInstall}</code>
                  </div>
                </div>
              </div>
            </div>
            <div className="setup-actions">
              <button className="setup-btn-primary" onClick={handleInstall}>
                {tr.installAutomatically}
              </button>
              <button className="setup-btn-ghost" onClick={onSkip}>
                {tr.skipManualConfigure}
              </button>
            </div>
          </div>
        )}

        {/* Installing / done / error */}
        {(phase === 'installing' || phase === 'done' || phase === 'error') && (
          <div className="setup-body">
            <div className="setup-log" ref={logRef}>
              {lines.map((l, i) => (
                <div key={i} className={`log-line log-${l.type}`}>
                  <span className="log-prefix">
                    {l.type === 'success' ? '✓' : l.type === 'error' ? '✗' : '›'}
                  </span>
                  {l.message}
                </div>
              ))}
              {phase === 'installing' && (
                <div className="log-line log-info">
                  <span className="log-spinner">⟳</span>
                  {tr.installing}
                </div>
              )}
            </div>

            {phase === 'done' && (
              <div className="setup-actions">
                <button
                  className="setup-btn-primary"
                  onClick={async () => {
                    const s = (await window.api.setup.check()) as SetupStatus
                    if (s.vibAcpPath) onComplete(s.vibAcpPath)
                  }}
                >
                  {tr.doneLaunchApp}
                </button>
              </div>
            )}

            {phase === 'error' && (
              <div className="setup-actions">
                <button className="setup-btn-primary" onClick={handleInstall}>
                  {tr.tryAgain}
                </button>
                <button className="setup-btn-ghost" onClick={onSkip}>
                  {tr.skipManualConfigure}
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
