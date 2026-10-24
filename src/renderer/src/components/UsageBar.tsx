import React from 'react'
import { useStore } from '../store'
import { cacheHitRatio } from '../types'

/**
 * Live usage bar below the input card: context fill, cache hit ratio,
 * in/out tokens, throughput and cost for the current session.
 * Compact single-line layout with short number formats (105.6M, 424.5k).
 */
function fmtTokens(n: number): string {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M'
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k'
  return String(n)
}

export default function UsageBar(): React.JSX.Element | null {
  const { usage, currentSessionId, config } = useStore()
  const u = currentSessionId ? usage[currentSessionId] : undefined
  if (!u) return null

  const hasContext = u.contextSize > 0
  const ctxPct = hasContext ? Math.min(100, (u.contextUsed / u.contextSize) * 100) : 0
  const hit = cacheHitRatio(u)
  const hitPct = hit !== null ? Math.round(hit * 100) : null
  const pl = config.language === 'pl'

  return (
    <div className="usage-bar">
      {hasContext && (
        <span
          className="usage-bar-item usage-context"
          title={pl
            ? 'Kontekst: ' + (u.contextUsed.toLocaleString()) + ' / ' + (u.contextSize.toLocaleString()) + ' tokenów'
            : 'Context: ' + (u.contextUsed.toLocaleString()) + ' / ' + (u.contextSize.toLocaleString()) + ' tokens'}
        >
          <span className="usage-bar-label">ctx</span>
          <span className="usage-context-track">
            <span
              className={'usage-context-fill' + (ctxPct > 85 ? ' warn' : '')}
              style={{ width: ctxPct + '%' }}
            />
          </span>
          <span className="usage-bar-value">{Math.round(ctxPct)}%</span>
        </span>
      )}
      {hitPct !== null && (
        <span
          className="usage-bar-item usage-cache"
          title={pl
            ? 'Cache: ' + ((u.cachedTokens ?? 0).toLocaleString()) + ' z ' + ((u.promptTokens ?? 0).toLocaleString()) + ' tokenów wejścia'
            : 'Cache: ' + ((u.cachedTokens ?? 0).toLocaleString()) + ' of ' + ((u.promptTokens ?? 0).toLocaleString()) + ' prompt tokens'}
        >
          <span className="usage-bar-label">⚡</span>
          <span className="usage-bar-value">{hitPct}%</span>
        </span>
      )}
      {u.promptTokens !== undefined && (
        <span className="usage-bar-item" title={pl ? 'Tokeny wejściowe' : 'Prompt tokens'}>
          <span className="usage-bar-label">in</span>
          <span className="usage-bar-value">{fmtTokens(u.promptTokens)}</span>
        </span>
      )}
      {u.completionTokens !== undefined && (
        <span className="usage-bar-item" title={pl ? 'Tokeny wyjściowe' : 'Completion tokens'}>
          <span className="usage-bar-label">out</span>
          <span className="usage-bar-value">{fmtTokens(u.completionTokens)}</span>
        </span>
      )}
      {u.tokensPerSecond !== undefined && u.tokensPerSecond > 0 && (
        <span className="usage-bar-item" title={pl ? 'Tokeny na sekundę' : 'Tokens per second'}>
          <span className="usage-bar-label">tok/s</span>
          <span className="usage-bar-value">{Math.round(u.tokensPerSecond)}</span>
        </span>
      )}
      {u.cost !== undefined && (
        <span
          className="usage-bar-item usage-cost"
          title={pl ? 'Koszt sesji: $' + u.cost.toFixed(4) : 'Session cost: $' + u.cost.toFixed(4)}
        >
          <span className="usage-bar-label">$</span>
          <span className="usage-bar-value">{u.cost.toFixed(2)}</span>
        </span>
      )}
    </div>
  )
}
