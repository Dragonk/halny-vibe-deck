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

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
)