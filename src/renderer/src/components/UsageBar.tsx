import React from 'react'
import { useStore } from '../store'
import { cacheHitRatio } from '../types'
import { useTranslation, t } from '../i18n'

/**
 * Live usage bar below the input card: context fill, cache hit ratio,
 * in/out tokens, throughput and cost for the current session.
 * Compact single-line layout with short number formats (105.6M, 424.5k).
 * v0.7.2: tooltips go through the i18n system (all 6 languages) instead
 * of the hardcoded pl/en ternaries.
 */
function fmtTokens(n: number): string {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M'
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k'
  return String(n)
}

export default function UsageBar(): React.JSX.Element | null {
  const { usage, currentSessionId, config } = useStore()
  const tr = useTranslation(config.language)
  const u = currentSessionId ? usage[currentSessionId] : undefined
  if (!u) return null

  const hasContext = u.contextSize > 0
  const ctxPct = hasContext ? Math.min(100, (u.contextUsed / u.contextSize) * 100) : 0
  const hit = cacheHitRatio(u)
  const hitPct = hit !== null ? Math.round(hit * 100) : null

  return (
    <div className="usage-bar">
      {hasContext && (
        <span
          className="usage-bar-item usage-context"
          title={t(tr, 'usageContextTooltip', {
            used: u.contextUsed.toLocaleString(),
            total: u.contextSize.toLocaleString(),
          })}
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
          title={t(tr, 'usageCacheTooltip', {
            cached: (u.cachedTokens ?? 0).toLocaleString(),
            total: (u.promptTokens ?? 0).toLocaleString(),
          })}
        >
          <span className="usage-bar-label">&#9889;</span>
          <span className="usage-bar-value">{hitPct}%</span>
        </span>
      )}
      {u.promptTokens !== undefined && (
        <span className="usage-bar-item" title={t(tr, 'usagePromptTooltip')}>
          <span className="usage-bar-label">in</span>
          <span className="usage-bar-value">{fmtTokens(u.promptTokens)}</span>
        </span>
      )}
      {u.completionTokens !== undefined && (
        <span className="usage-bar-item" title={t(tr, 'usageCompletionTooltip')}>
          <span className="usage-bar-label">out</span>
          <span className="usage-bar-value">{fmtTokens(u.completionTokens)}</span>
        </span>
      )}
      {u.tokensPerSecond !== undefined && u.tokensPerSecond > 0 && (
        <span className="usage-bar-item" title={t(tr, 'usageTpsTooltip')}>
          <span className="usage-bar-label">tok/s</span>
          <span className="usage-bar-value">{Math.round(u.tokensPerSecond)}</span>
        </span>
      )}
      {u.cost !== undefined && (
        <span
          className="usage-bar-item usage-cost"
          title={t(tr, 'usageCostTooltip', { cost: u.cost.toFixed(4) })}
        >
          <span className="usage-bar-label">$</span>
          <span className="usage-bar-value">{u.cost.toFixed(2)}</span>
        </span>
      )}
    </div>
  )
}
