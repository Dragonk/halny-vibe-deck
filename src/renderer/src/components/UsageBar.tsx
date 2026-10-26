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
 * v0.10.2: the linear context bar becomes a Zed-style circular gauge -
 * real used context vs the model's context window (runtime window from
 * usage_update; config max_context_length fills in for previewed
 * sessions) with a marker at the CLI's auto_compact_threshold.
 */
function fmtTokens(n: number): string {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M'
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k'
  return String(n)
}

/**
 * Mirror of the CLI's auto_compact_threshold resolution
 * (vibe/core/config/vibe_schema.py): per-model cap wins, then the global
 * cap, then 80% of the declared window, then 200 000. 0 = never compact.
 */
function resolveCompactThreshold(
  model: { auto_compact_threshold?: number; max_context_length?: number } | undefined,
  globalThreshold: number | undefined
): number | null {
  const cap = model?.auto_compact_threshold
  const window = model?.max_context_length ?? null
  if (cap !== undefined && cap !== -1 && (window === null || cap < window)) return cap
  const globalSet = globalThreshold !== undefined && globalThreshold !== -1
  const globalValue = globalSet ? (globalThreshold as number) : 200000
  if (globalSet && (window === null || globalValue < window)) return globalValue
  if (window) return Math.trunc(window * 0.8)
  return globalValue
}

interface VibeConfigLike {
  models?: Array<{
    name: string
    alias?: string
    auto_compact_threshold?: number
    max_context_length?: number
  }>
  auto_compact_threshold?: number
}

const GAUGE_R = 6
const GAUGE_CIRC = 2 * Math.PI * GAUGE_R

export default function UsageBar(): React.JSX.Element | null {
  const { usage, currentSessionId, config } = useStore()
  const tr = useTranslation(config.language)
  const currentModel = useStore((s) => s.currentModel)
  const sessions = useStore((s) => s.sessions)
  const currentSess = currentSessionId ? sessions.find((s) => s.id === currentSessionId) : undefined
  const u = currentSessionId ? usage[currentSessionId] : undefined

  const sessWd = currentSess?.workingDirectory ?? ''
  const sessRid = currentSess?.remoteId ?? ''
  const [modelInfo, setModelInfo] = React.useState<{
    window: number | null
    threshold: number | null
  } | null>(null)

  React.useEffect(() => {
    let cancelled = false
    const load = async (): Promise<void> => {
      try {
        if (sessRid) {
          const cfg = (await window.api.vibe.getConfig('remote', '', sessRid)) as VibeConfigLike
          if (cancelled) return
          const model = (cfg.models ?? []).find(
            (m) => m.name === currentModel || m.alias === currentModel
          )
          setModelInfo({
            window: model?.max_context_length ?? null,
            threshold: model
              ? resolveCompactThreshold(model, cfg.auto_compact_threshold)
              : null
          })
          return
        }
        const gLoad = window.api.vibe.getConfig('global', sessWd)
        const pLoad = window.api.vibe.getConfig('project', sessWd)
        const [gcfg, pcfg] = (await Promise.all([gLoad, pLoad])) as [VibeConfigLike, VibeConfigLike]
        if (cancelled) return
        // Project entries win over global ones with the same name.
        const models = [...(pcfg.models ?? []), ...(gcfg.models ?? [])]
        const model = models.find((m) => m.name === currentModel || m.alias === currentModel)
        setModelInfo({
          window: model?.max_context_length ?? null,
          threshold: model
            ? resolveCompactThreshold(
                model,
                pcfg.auto_compact_threshold ?? gcfg.auto_compact_threshold
              )
            : null
        })
      } catch {
        /* config unavailable - the gauge falls back to runtime numbers */
      }
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [currentModel, sessWd, sessRid])

  if (!u) {
    // v0.13.14: keep the slot visible (the web client reloads often and the
    // bar must not vanish before the first usage_update arrives).
    return (
      <div className="usage-bar">
        <span className="usage-bar-item">
          <svg className="usage-gauge" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <circle className="usage-gauge-track" cx="8" cy="8" r={GAUGE_R} />
          </svg>
        </span>
      </div>
    );
  }

  // v0.13.5: the real max context (window) comes ONLY from the model config
  // (max_context_length). The runtime usage_update.size is the CLI-resolved
  // AUTO-COMPACT THRESHOLD, not the window - using it as the denominator
  // would conflate the compact limit with the real context size. A model
  // that declares no window simply has no gauge (tokens still show).
  const ctxWindow = modelInfo?.window ?? null
  const hasContext = ctxWindow !== null && ctxWindow > 0
  const ctxPct = hasContext ? Math.min(100, (u.contextUsed / (ctxWindow as number)) * 100) : 0
  const threshold = modelInfo?.threshold ?? null
  const compactPct =
    hasContext && threshold !== null && threshold > 0
      ? Math.min(100, (threshold / (ctxWindow as number)) * 100)
      : null
  const overCompact = compactPct !== null && ctxPct >= compactPct
  const hit = cacheHitRatio(u)
  const hitPct = hit !== null ? Math.round(hit * 100) : null

  let gaugeTitle: string | undefined
  if (hasContext) {
    if (threshold !== null && threshold > 0) {
      gaugeTitle = t(tr, 'usageGaugeTooltip', {
        used: u.contextUsed.toLocaleString(),
        window: (ctxWindow as number).toLocaleString(),
        compact: threshold.toLocaleString()
      })
    } else {
      gaugeTitle =
        t(tr, 'usageContextTooltip', {
          used: u.contextUsed.toLocaleString(),
          total: (ctxWindow as number).toLocaleString()
        }) +
        ' - ' +
        t(tr, 'usageCompactOff')
    }
  }

  return (
    <div className="usage-bar">
      {hasContext && (
        <span className="usage-bar-item usage-context" title={gaugeTitle}>
          <svg className="usage-gauge" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
            <g transform="rotate(-90 8 8)">
              <circle className="usage-gauge-track" cx="8" cy="8" r={GAUGE_R} />
              <circle
                className={'usage-gauge-fill' + (overCompact ? ' warn' : '')}
                cx="8"
                cy="8"
                r={GAUGE_R}
                strokeDasharray={(ctxPct / 100) * GAUGE_CIRC + ' ' + GAUGE_CIRC}
              />
              {compactPct !== null && compactPct < 99.5 && (
                <circle
                  className="usage-gauge-mark"
                  cx="8"
                  cy="8"
                  r={GAUGE_R}
                  strokeDasharray={'1 ' + GAUGE_CIRC}
                  strokeDashoffset={-(compactPct / 100) * GAUGE_CIRC}
                />
              )}
            </g>
          </svg>
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
