import React from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'

export default function Header(): React.JSX.Element {
  const { theme, toggleTheme, acpStatus, acpError, config } = useStore()
  const tr = useTranslation(config.language)

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
        <div className="header-brand">
          <img src="/icon.png" alt="" className="header-brand-img" />
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
            {acpStatus === 'connected' && <div style={{ marginTop: 2 }}>{tr.agentLabel}</div>}
            {acpStatus === 'error' && acpError && <div style={{ marginTop: 2, color: '#f87171' }}>{acpError}</div>}
          </div>
        </div>
      </div>
      <div
        className="header-controls"
        style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
      >
        <button
          className="header-btn theme-btn"
          onClick={toggleTheme}
          title={getThemeTitle()}
        >
          {getThemeIcon()}
        </button>
        <button
          className="header-btn"
          onClick={() => window.api.window.minimize()}
          title={tr.minimize}
        >
          ─
        </button>
        <button
          className="header-btn"
          onClick={() => window.api.window.maximize()}
          title={tr.maximize}
        >
          □
        </button>
        <button
          className="header-btn close-btn"
          onClick={() => window.api.window.close()}
          title={tr.close}
        >
          ✕
        </button>
      </div>
    </header>
  )
}
