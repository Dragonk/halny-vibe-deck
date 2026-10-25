import React from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import { isWebMode } from '../webBridge'

export default function Header(): React.JSX.Element {
  const {
    theme,
    toggleTheme,
    acpStatus,
    acpError,
    config,
    dockOpen,
    setDockOpen,
    mobileSidebarOpen,
    setMobileSidebarOpen,
  } = useStore()
  const tr = useTranslation(config.language)
  const web = isWebMode()

  const handleLogout = async (): Promise<void> => {
    try {
      await fetch('/api/auth/logout', { method: 'POST' })
    } catch {
      /* best effort */
    }
    window.location.href = '/login'
  }

  const getThemeIcon = () => {
    if (theme === 'auto') return '◑'
    return theme === 'dark' ? '☀' : '☾'
  }

  const getThemeTitle = () => {
    if (theme === 'auto') return tr.themeAuto
    if (theme === 'dark') return tr.themeDark
    return tr.themeLight
  }

  const getActivityStatus = () => {
    switch (acpStatus) {
      case 'connected':
        return { text: tr.statusConnected, className: 'activity-connected' }
      case 'connecting':
        return { text: tr.statusConnecting, className: 'activity-connecting' }
      case 'error':
        return { text: t(tr, 'statusError', { error: acpError || '' }), className: 'activity-error' }
      default:
        return { text: tr.statusDisconnected, className: 'activity-disconnected' }
    }
  }

  const activity = getActivityStatus()

  return (
    <header className="header" style={{ WebkitAppRegion: 'drag' } as React.CSSProperties}>
      <div className="header-left">
        <button
          className="header-btn header-menu-btn"
          style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
          onClick={() => setMobileSidebarOpen(!mobileSidebarOpen)}
          title={tr.menuLabel}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          >
            <line x1="3" y1="6" x2="21" y2="6" />
            <line x1="3" y1="12" x2="21" y2="12" />
            <line x1="3" y1="18" x2="21" y2="18" />
          </svg>
        </button>
        <div className="header-brand">
          <img src="./icon.png" alt="" className="header-brand-img" />
          <span className="header-brand-name">{tr.appName}</span>
        </div>
        <div
          className={`header-activity ${activity.className}`}
          style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
        >
          <span className="activity-icon" />
          <span className="activity-text">{activity.text}</span>
          <div className="activity-tooltip chart-tooltip">
            <div>{t(tr, 'statusLabel', { status: activity.text })}</div>
            {acpStatus === 'connected' && (
              <div style={{ marginTop: 2 }}>{tr.agentLabel}</div>
            )}
            {acpStatus === 'error' && acpError && (
              <div style={{ marginTop: 2, color: '#f87171' }}>{acpError}</div>
            )}
          </div>
        </div>
      </div>
      <div
        className="header-controls"
        style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
      >
        <button
          className={`header-btn dock-toggle-btn ${dockOpen ? 'active' : ''}`}
          onClick={() => setDockOpen(!dockOpen)}
          title={tr.dockToggle}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="15" y1="3" x2="15" y2="21" />
          </svg>
        </button>
        <button className="header-btn theme-btn" onClick={toggleTheme} title={getThemeTitle()}>
          {getThemeIcon()}
        </button>
        {!web && (
          <>
            <button className="header-btn" onClick={() => window.api.window.minimize()} title={tr.minimize}>
              ─
            </button>
            <button className="header-btn" onClick={() => window.api.window.maximize()} title={tr.maximize}>
              □
            </button>
            <button className="header-btn close-btn" onClick={() => window.api.window.close()} title={tr.close}>
              ✕
            </button>
          </>
        )}
        {web && (
          <button className="header-btn" onClick={() => void handleLogout()} title={tr.logoutLabel}>
            ⏻
          </button>
        )}
      </div>
    </header>
  )
}
