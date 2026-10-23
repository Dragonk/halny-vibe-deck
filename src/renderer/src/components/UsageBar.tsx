import React from 'react'
import { useStore } from '../store'
import { cacheHitRatio } from '../types'

/**
 * Live usage bar below the input card: context fill, cache hit ratio,
 * cached/prompt tokens, cost and throughput for the current session.
 */
export default function UsageBar(): React.JSX.Element | null {
  const { usage, currentSessionId, config } = useStore()
  const u = currentSessionId ? usage[currentSessionId] : undefined
  if (!u) return null

  const hasContext = u.contextSize > 0
  const ctxPct = hasContext ? Math.min(100, (u.contextUsed / u.contextSize) * 100) : 0
  const hit = cacheHitRatio(u)
  const hitPct = hit !== null ? Math.round(hit * 100) : null

  return (
    <div className="usage-bar">
      {hasContext && (
        <div className="usage-bar-item usage-context" title={`${u.contextUsed} / ${u.contextSize}`}>
          <span className="usage-bar-label">ctx</span>
          <span className="usage-context-track">
            <span
              className={`usage-context-fill ${ctxPct > 85 ? 'warn' : ''}`}
              style={{ width: `${ctxPct}%` }}
            />
          </span>
          <span className="usage-bar-value">{Math.round(ctxPct)}%</span>
        </div>
      )}
      {hitPct !== null && (
        <div
          className="usage-bar-item usage-cache"
          title={`${(u.cachedTokens ?? 0).toLocaleString()} cached / ${(u.promptTokens ?? 0).toLocaleString()} prompt tokens`}
        >
          <span className="usage-bar-label">⚡ cache</span>
          <span className="usage-bar-value">{hitPct}%</span>
        </div>
      )}
      {u.cachedTokens !== undefined && (
        <div className="usage-bar-item" title={config.language === 'pl' ? 'Tokeny z cache' : 'Cached tokens'}>
          <span className="usage-bar-label">cached</span>
          <span className="usage-bar-value">{u.cachedTokens.toLocaleString()}</span>
        </div>
      )}
      {u.promptTokens !== undefined && (
        <div className="usage-bar-item" title="Prompt tokens">
          <span className="usage-bar-label">in</span>
          <span className="usage-bar-value">{u.promptTokens.toLocaleString()}</span>
        </div>
      )}
      {u.completionTokens !== undefined && (
        <div className="usage-bar-item" title="Completion tokens">
          <span className="usage-bar-label">out</span>
          <span className="usage-bar-value">{u.completionTokens.toLocaleString()}</span>
        </div>
      )}
      {u.tokensPerSecond !== undefined && u.tokensPerSecond > 0 && (
        <div className="usage-bar-item" title="Tokens per second">
          <span className="usage-bar-label">tok/s</span>
          <span className="usage-bar-value">{Math.round(u.tokensPerSecond)}</span>
        </div>
      )}
      {u.cost !== undefined && (
        <div className="usage-bar-item usage-cost" title="Session cost">
          <span className="usage-bar-label">$</span>
          <span className="usage-bar-value">{u.cost.toFixed(4)}</span>
        </div>
      )}
    </div>
  )
}
