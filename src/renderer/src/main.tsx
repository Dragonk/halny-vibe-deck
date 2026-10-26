import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './styles/globals.css'

// Top-level safety net: if any component throws during render, show the
// error on screen instead of leaving a blank (black) window.
class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null }

  static getDerivedStateFromError(error: Error): { error: Error | null } {
    return { error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    console.error('[renderer] uncaught error:', error, info)
  }

  render(): React.ReactNode {
    if (this.state.error) {
      return (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            padding: '32px',
            overflow: 'auto',
            background: '#111115',
            color: '#ff6b6b',
            fontFamily: 'monospace',
            fontSize: '13px',
            lineHeight: 1.5,
          }}
        >
          <div style={{ color: '#fff', fontSize: '16px', marginBottom: '12px' }}>
            Something went wrong while rendering the app.
          </div>
          <div style={{ color: '#aaa', marginBottom: '16px' }}>
            Wystąpił nieoczekiwany błąd aplikacji. Poniżej szczegóły błędu.
          </div>
          <pre style={{ whiteSpace: 'pre-wrap', margin: 0 }}>
            {this.state.error.stack || this.state.error.message}
          </pre>
          <button
            type='button'
            onClick={() => window.location.reload()}
            style={{
              marginTop: '16px',
              padding: '8px 16px',
              background: '#5b6cff',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              cursor: 'pointer',
            }}
          >
            Reload
          </button>
        </div>
      )
    }
    return this.props.children
  }
}

async function boot(): Promise<void> {
  // Web mode (UI served over HTTPS by serve mode): preload does not exist,
  // so we rebuild window.api on top of the /ws websocket bridge.
  const w = window as unknown as { api?: unknown; __NEXUS_WEB__?: boolean }
  if (!w.api) {
    const webBridge = await import('./webBridge')
    await webBridge.initWebApi()
  }
  // PWA service worker — only meaningful in web mode (HTTPS origin).
  if (w.__NEXUS_WEB__ && 'serviceWorker' in navigator) {
    try {
      await navigator.serviceWorker.register('/sw.js')
      // v1.0.10: when a new service worker takes over (app update), reload
      // once so the phone never keeps running the previous bundle.
      let reloaded = false
      navigator.serviceWorker.addEventListener('controllerchange', () => {
        if (reloaded) return
        reloaded = true
        window.location.reload()
      })
    } catch (err) {
      console.warn('[renderer] service worker registration failed:', err)
    }
  }
  ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  )
}

void boot()
