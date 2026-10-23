import React, { useEffect, useState, useMemo, useRef, useCallback } from 'react'
import { useStore } from '../store'
import { useTranslation, t } from '../i18n'
import type { VibeSession } from '../types'

type TimeRange = '7d' | '30d' | '90d' | 'all' | 'custom'

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`
  if (secs < 3600) return `${Math.round(secs / 60)}m`
  return `${(secs / 3600).toFixed(1)}h`
}

function dayLabel(date: Date): string {
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

function fmtYTokens(v: number): string {
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`
  if (v >= 1_000) return `${(v / 1_000).toFixed(0)}k`
  return String(Math.round(v))
}

function fmtYCost(v: number): string {
  if (v >= 10) return `$${v.toFixed(0)}`
  return `$${v.toFixed(2)}`
}

function ChartTooltip({ x, y, lines }: { x: number; y: number; lines: string[] }): React.JSX.Element {
  return (
    <div
      className="chart-tooltip"
      style={{ position: 'fixed', left: x + 12, top: y - 8, pointerEvents: 'none', zIndex: 9999 }}
    >
      {lines.map((l, i) => <div key={i}>{l}</div>)}
    </div>
  )
}

interface TooltipState {
  x: number
  y: number
  lines: string[]
}

// ───── Area chart with axes + tooltip ─────
function AreaChart({
  data,
  color,
  formatY = fmtYCost,
  emptyLabel = 'Not enough data'
}: {
  data: { x: number; y: number }[]
  color: string
  formatY?: (v: number) => string
  emptyLabel?: string
}): React.JSX.Element {
  const [tooltip, setTooltip] = useState<TooltipState | null>(null)

  if (data.length < 2) return <div className="chart-empty">{emptyLabel}</div>

  const W = 300
  const H = 130
  const pad = { l: 44, r: 8, t: 8, b: 22 }
  const cW = W - pad.l - pad.r
  const cH = H - pad.t - pad.b

  const xs = data.map((d) => d.x)
  const ys = data.map((d) => d.y)
  const minX = Math.min(...xs)
  const maxX = Math.max(...xs)
  const maxY = Math.max(...ys) || 1

  const sx = (x: number): number => pad.l + ((x - minX) / (maxX - minX || 1)) * cW
  const sy = (y: number): number => pad.t + cH - (y / maxY) * cH

  const pts = data.map((d) => `${sx(d.x)},${sy(d.y)}`).join(' ')
  const area = `M${sx(xs[0])},${sy(0)} L${pts} L${sx(xs[xs.length - 1])},${sy(0)} Z`
  const gradId = `ag-${color.replace(/[^a-z0-9]/gi, '')}`

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((t) => ({ v: t * maxY, sy: sy(t * maxY) }))

  // Pick x-axis tick indices by minimum pixel spacing, not fixed stride
  const MIN_X_GAP = 40
  const xTickIndices: number[] = []
  let lastTickX = -999
  data.forEach((d, i) => {
    const px = sx(d.x)
    if (px - lastTickX >= MIN_X_GAP) {
      xTickIndices.push(i)
      lastTickX = px
    }
  })
  // Ensure last point shown; replace last tick if it overlaps, else append
  if (data.length > 0) {
    const lastIdx = data.length - 1
    const lastPx = sx(data[lastIdx].x)
    if (!xTickIndices.includes(lastIdx)) {
      const prevPx = xTickIndices.length > 0 ? sx(data[xTickIndices[xTickIndices.length - 1]].x) : -999
      if (lastPx - prevPx < MIN_X_GAP / 2) {
        xTickIndices[xTickIndices.length - 1] = lastIdx
      } else {
        xTickIndices.push(lastIdx)
      }
    }
  }

  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement>): void => {
    const rect = e.currentTarget.getBoundingClientRect()
    const mouseX = ((e.clientX - rect.left) / rect.width) * W
    const closestIdx = data.reduce((best, _, i) => {
      return Math.abs(sx(xs[i]) - mouseX) < Math.abs(sx(xs[best]) - mouseX) ? i : best
    }, 0)
    const d = data[closestIdx]
    setTooltip({
      x: e.clientX,
      y: e.clientY,
      lines: [
        new Date(d.x).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }),
        formatY(d.y)
      ]
    })
  }

  return (
    <div style={{ position: 'relative', height: '100%' }}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="chart-svg"
        preserveAspectRatio="xMidYMid meet"
        onMouseMove={handleMouseMove}
        onMouseLeave={() => setTooltip(null)}
        style={{ cursor: 'crosshair' }}
      >
        <defs>
          <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.35" />
            <stop offset="100%" stopColor={color} stopOpacity="0.03" />
          </linearGradient>
        </defs>
        {yTicks.map((t, i) => (
          <line key={i} x1={pad.l} y1={t.sy} x2={W - pad.r} y2={t.sy} stroke="var(--border-subtle)" strokeWidth="0.5" />
        ))}
        <path d={area} fill={`url(#${gradId})`} />
        <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" />
        {yTicks.map((t, i) => (
          <text key={i} x={pad.l - 4} y={t.sy + 3} textAnchor="end" fontSize="8" fill="var(--text-muted)">{formatY(t.v)}</text>
        ))}
        {xTickIndices.map((i) => {
          const d = data[i]
          return <text key={i} x={sx(d.x)} y={H - 4} textAnchor="middle" fontSize="8" fill="var(--text-muted)">{new Date(d.x).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</text>
        })}
      </svg>
      {tooltip && <ChartTooltip x={tooltip.x} y={tooltip.y} lines={tooltip.lines} />}
    </div>
  )
}

// ───── Bar chart with axes + tooltip ─────
function BarChart({
  data,
  colors,
  legend,
  emptyLabel = 'Not enough data'
}: {
  data: { label: string; values: number[]; date: number }[]
  colors: string[]
  legend?: string[]
  emptyLabel?: string
}): React.JSX.Element {
  const [tooltip, setTooltip] = useState<TooltipState | null>(null)

  if (data.length === 0) return <div className="chart-empty">{emptyLabel}</div>

  const W = 300
  const H = 130
  const pad = { l: 44, r: 8, t: 8, b: 22 }
  const cW = W - pad.l - pad.r
  const cH = H - pad.t - pad.b

  const totals = data.map((d) => d.values.reduce((a, b) => a + b, 0))
  const maxY = Math.max(...totals) || 1
  const barW = Math.max(4, (cW / data.length) * 0.7)
  const gap = cW / data.length

  const sy = (y: number): number => pad.t + cH - (y / maxY) * cH

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((t) => ({
    v: t * maxY,
    sy: sy(t * maxY)
  }))

  // Pick bar x-axis tick indices by minimum pixel spacing
  const BAR_MIN_X_GAP = 40
  const barTickIndices: number[] = []
  let lastBarTickX = -999
  data.forEach((d, i) => {
    const px = pad.l + i * gap + gap * 0.5
    if (px - lastBarTickX >= BAR_MIN_X_GAP) {
      barTickIndices.push(i)
      lastBarTickX = px
    }
  })
  if (data.length > 0) {
    const lastIdx = data.length - 1
    const lastPx = pad.l + lastIdx * gap + gap * 0.5
    if (!barTickIndices.includes(lastIdx)) {
      const prevPx = barTickIndices.length > 0
        ? pad.l + barTickIndices[barTickIndices.length - 1] * gap + gap * 0.5
        : -999
      if (lastPx - prevPx < BAR_MIN_X_GAP / 2) {
        barTickIndices[barTickIndices.length - 1] = lastIdx
      } else {
        barTickIndices.push(lastIdx)
      }
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', gap: 6, position: 'relative' }}>
      {legend && legend.length > 1 && (
        <div style={{ display: 'flex', gap: 10, paddingLeft: 4 }}>
          {legend.map((l, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <span style={{ width: 8, height: 8, borderRadius: 2, background: colors[i % colors.length], flexShrink: 0, display: 'inline-block' }} />
              <span style={{ fontSize: 10, color: 'var(--text-muted)' }}>{l}</span>
            </div>
          ))}
        </div>
      )}
      <svg viewBox={`0 0 ${W} ${H}`} className="chart-svg" preserveAspectRatio="xMidYMid meet" style={{ flex: 1 }} onMouseLeave={() => setTooltip(null)}>
        {yTicks.map((t, i) => (
          <line key={i} x1={pad.l} y1={t.sy} x2={W - pad.r} y2={t.sy} stroke="var(--border-subtle)" strokeWidth="0.5" />
        ))}
        {data.map((d, i) => {
          const x = pad.l + i * gap + gap * 0.15
          let yOff = 0
          return d.values.map((v, vi) => {
            const h = (v / maxY) * cH
            const y = pad.t + cH - yOff - h
            yOff += h
            return (
              <rect
                key={vi}
                x={x} y={y} width={barW} height={Math.max(h, 0)} rx="1"
                fill={colors[vi % colors.length]}
                style={{ cursor: 'pointer' }}
                onMouseEnter={(e) => {
                  const tipLines = [
                    d.label,
                    ...(legend
                      ? d.values.map((val, j) => `${legend[j] ?? j}: ${formatTokens(val)}`)
                      : [`${formatTokens(v)}`])
                  ]
                  setTooltip({ x: e.clientX, y: e.clientY, lines: tipLines })
                }}
                onMouseLeave={() => setTooltip(null)}
              />
            )
          })
        })}
        {yTicks.map((t, i) => (
          <text key={i} x={pad.l - 4} y={t.sy + 3} textAnchor="end" fontSize="8" fill="var(--text-muted)">{fmtYTokens(t.v)}</text>
        ))}
        {barTickIndices.map((i) => {
          const d = data[i]
          return <text key={i} x={pad.l + i * gap + gap * 0.5} y={H - 4} textAnchor="middle" fontSize="8" fill="var(--text-muted)">{d.label}</text>
        })}
      </svg>
      {tooltip && <ChartTooltip x={tooltip.x} y={tooltip.y} lines={tooltip.lines} />}
    </div>
  )
}

// ───── Donut chart with tooltip ─────
function DonutChart({
  data,
  size = 100,
  emptyLabel = 'No data',
  totalLabel = 'total'
}: {
  data: { label: string; value: number; color: string }[]
  size?: number
  emptyLabel?: string
  totalLabel?: string
}): React.JSX.Element {
  const [tooltip, setTooltip] = useState<TooltipState | null>(null)
  const total = data.reduce((s, d) => s + d.value, 0)
  if (total === 0) return <div className="chart-empty">{emptyLabel}</div>

  const cx = 50
  const cy = 50
  const r = 40
  const inner = 24
  let angle = -Math.PI / 2

  const arcs = data
    .filter((d) => d.value > 0)
    .map((d) => {
      const sweep = (d.value / total) * 2 * Math.PI
      const x1 = cx + r * Math.cos(angle)
      const y1 = cy + r * Math.sin(angle)
      angle += sweep
      const x2 = cx + r * Math.cos(angle)
      const y2 = cy + r * Math.sin(angle)
      const lx1 = cx + inner * Math.cos(angle - sweep)
      const ly1 = cy + inner * Math.sin(angle - sweep)
      const lx2 = cx + inner * Math.cos(angle)
      const ly2 = cy + inner * Math.sin(angle)
      const large = sweep > Math.PI ? 1 : 0
      return {
        d: `M${x1},${y1} A${r},${r} 0 ${large},1 ${x2},${y2} L${lx2},${ly2} A${inner},${inner} 0 ${large},0 ${lx1},${ly1} Z`,
        color: d.color,
        label: d.label,
        value: d.value
      }
    })

  return (
    <div style={{ position: 'relative', display: 'inline-block' }}>
      <svg
        viewBox="0 0 100 100"
        width={size}
        height={size}
        style={{ display: 'block' }}
        onMouseLeave={() => setTooltip(null)}
      >
        {arcs.map((a, i) => (
          <path
            key={i} d={a.d} fill={a.color} strokeWidth="0.5" stroke="var(--bg)"
            style={{ cursor: 'pointer', transition: 'opacity 0.1s' }}
            onMouseEnter={(e) => setTooltip({ x: e.clientX, y: e.clientY, lines: [`${a.label}`, `${a.value} (${Math.round((a.value / total) * 100)}%)`] })}
            onMouseLeave={() => setTooltip(null)}
          />
        ))}
        <text x={cx} y={cy - 4} textAnchor="middle" fontSize="10" fill="var(--text-muted)">{totalLabel}</text>
        <text x={cx} y={cy + 8} textAnchor="middle" fontSize="13" fontWeight="600" fill="var(--text)">{total}</text>
      </svg>
      {tooltip && <ChartTooltip x={tooltip.x} y={tooltip.y} lines={tooltip.lines} />}
    </div>
  )
}

// ───── Heatmap ─────
function Heatmap({ sessions, noSessionsLabel = 'No sessions', sessionLabel = 'session' }: { sessions: VibeSession[]; noSessionsLabel?: string; sessionLabel?: string }): React.JSX.Element {
  const [tooltip, setTooltip] = useState<TooltipState | null>(null)

  const countByDay = useMemo(() => {
    const map: Record<string, number> = {}
    for (const s of sessions) {
      const key = new Date(s.startTime).toISOString().slice(0, 10)
      map[key] = (map[key] ?? 0) + 1
    }
    return map
  }, [sessions])

  const today = new Date()
  const cells: { date: string; count: number }[] = []
  for (let i = 89; i >= 0; i--) {
    const d = new Date(today)
    d.setDate(d.getDate() - i)
    const key = d.toISOString().slice(0, 10)
    cells.push({ date: key, count: countByDay[key] ?? 0 })
  }

  const firstDay = new Date(cells[0].date).getDay()
  const padded = Array(firstDay).fill(null).concat(cells)

  const weeks: (typeof cells[0] | null)[][] = []
  for (let i = 0; i < padded.length; i += 7) {
    weeks.push(padded.slice(i, i + 7) as (typeof cells[0] | null)[])
  }

  const colorFor = (n: number): string => {
    if (n === 0) return 'var(--heatmap-0)'
    if (n === 1) return 'var(--heatmap-1)'
    if (n <= 3) return 'var(--heatmap-2)'
    if (n <= 6) return 'var(--heatmap-3)'
    return 'var(--heatmap-4)'
  }

  return (
    <div className="heatmap" style={{ position: 'relative' }}>
      {weeks.map((week, wi) => (
        <div key={wi} className="heatmap-col">
          {week.map((cell, di) =>
            cell ? (
              <div
                key={di}
                className="heatmap-cell"
                style={{ background: colorFor(cell.count), cursor: 'pointer' }}
                onMouseEnter={(e) =>
                  setTooltip({
                    x: e.clientX,
                    y: e.clientY,
                    lines: [
                      cell.date,
                      cell.count === 0 ? noSessionsLabel : `${cell.count} ${cell.count > 1 ? sessionLabel + 's' : sessionLabel}`
                    ]
                  })
                }
                onMouseMove={(e) =>
                  setTooltip((prev) => prev && { ...prev, x: e.clientX, y: e.clientY })
                }
                onMouseLeave={() => setTooltip(null)}
              />
            ) : (
              <div key={di} className="heatmap-cell heatmap-empty" />
            )
          )}
        </div>
      ))}
      {tooltip && <ChartTooltip x={tooltip.x} y={tooltip.y} lines={tooltip.lines} />}
    </div>
  )
}

// ───── Cards ─────

function CostCard({ sessions, tr }: { sessions: VibeSession[]; tr: ReturnType<typeof useTranslation> }): React.JSX.Element {
  const [selectedModel, setSelectedModel] = useState<string>('all')

  const models = useMemo(
    () => [...new Set(sessions.map((s) => s.config?.activeModel ?? 'unknown'))].sort(),
    [sessions]
  )

  const filtered =
    selectedModel === 'all'
      ? sessions
      : sessions.filter((s) => (s.config?.activeModel ?? 'unknown') === selectedModel)

  const totalCost = filtered.reduce((s, sess) => s + (sess.stats.sessionCost ?? 0), 0)
  const avgCost = filtered.length > 0 ? totalCost / filtered.length : 0

  const cumulativeData = useMemo(() => {
    let running = 0
    return filtered
      .slice()
      .sort((a, b) => new Date(a.startTime).getTime() - new Date(b.startTime).getTime())
      .map((s) => {
        running += s.stats.sessionCost ?? 0
        return { x: new Date(s.startTime).getTime(), y: running }
      })
  }, [filtered])

  return (
    <div className="stat-card">
      <div className="card-header">
        <div>
          <div className="card-subtitle">{tr.totalSpend}</div>
          <div className="card-value primary">${totalCost.toFixed(2)}</div>
        </div>
        <div className="card-meta">
          <div>{t(tr, 'sessionsCount', { count: filtered.length })}</div>
          <div>{tr.avg} ${avgCost.toFixed(2)}{tr.perSession}</div>
        </div>
      </div>
      {models.length > 1 && (
        <select
          className="stats-model-select"
          value={selectedModel}
          onChange={(e) => setSelectedModel(e.target.value)}
        >
          <option value="all">{tr.allModels}</option>
          {models.map((m) => (
            <option key={m} value={m}>{m}</option>
          ))}
        </select>
      )}
      <div className="card-chart">
        <AreaChart data={cumulativeData} color="var(--color-primary)" formatY={fmtYCost} />
      </div>
    </div>
  )
}

function TokenCard({ sessions, tr }: { sessions: VibeSession[]; tr: ReturnType<typeof useTranslation> }): React.JSX.Element {
  type Filter = 'both' | 'input' | 'output'
  const [filter, setFilter] = useState<Filter>('both')

  const totalInput = sessions.reduce((s, sess) => s + (sess.stats.sessionPromptTokens ?? 0), 0)
  const totalOutput = sessions.reduce(
    (s, sess) => s + (sess.stats.sessionCompletionTokens ?? 0),
    0
  )
  const totalBoth = totalInput + totalOutput
  const avgTps =
    sessions.length > 0
      ? sessions.reduce((s, sess) => s + (sess.stats.tokensPerSecond ?? 0), 0) / sessions.length
      : 0

  const displayTotal = filter === 'input' ? totalInput : filter === 'output' ? totalOutput : totalBoth

  const barData = useMemo(() => {
    return sessions
      .slice()
      .sort((a, b) => new Date(a.startTime).getTime() - new Date(b.startTime).getTime())
      .map((s) => ({
        date: new Date(s.startTime).getTime(),
        label: dayLabel(new Date(s.startTime)),
        values:
          filter === 'input'
            ? [s.stats.sessionPromptTokens ?? 0]
            : filter === 'output'
              ? [s.stats.sessionCompletionTokens ?? 0]
              : [s.stats.sessionPromptTokens ?? 0, s.stats.sessionCompletionTokens ?? 0]
      }))
  }, [sessions, filter])

  const barColors =
    filter === 'input'
      ? ['var(--color-accent)']
      : filter === 'output'
        ? ['var(--color-primary)']
        : ['var(--color-accent)', 'var(--color-primary)']

  const barLegend =
    filter === 'both'
      ? [tr.inputLabel, tr.outputLabel]
      : filter === 'input'
        ? [tr.inputLabel]
        : [tr.outputLabel]

  return (
    <div className="stat-card">
      <div className="card-header">
        <div>
          <div className="card-subtitle">{tr.totalTokens}</div>
          <div className="card-value accent">{formatTokens(displayTotal)}</div>
        </div>
        <div className="card-meta">
          <div>{avgTps.toFixed(0)} {tr.tokPerSecAvg}</div>
          <div>
            {formatTokens(totalInput)} {tr.inOut.split(' / ')[0]} / {formatTokens(totalOutput)} {tr.inOut.split(' / ')[1]}
          </div>
        </div>
      </div>
      <div className="pill-row">
        {(['both', 'input', 'output'] as Filter[]).map((f) => (
          <button
            key={f}
            className={`pill ${filter === f ? 'active' : ''}`}
            onClick={() => setFilter(f)}
          >
            {f === 'both' ? tr.both : f === 'input' ? tr.inputLabel : tr.outputLabel}
          </button>
        ))}
      </div>
      <div className="card-chart">
        <BarChart data={barData} colors={barColors} legend={barLegend} />
      </div>
    </div>
  )
}

function ActivityCard({ sessions, tr }: { sessions: VibeSession[]; tr: ReturnType<typeof useTranslation> }): React.JSX.Element {
  const totalDuration = sessions.reduce((s, sess) => {
    const start = new Date(sess.startTime).getTime()
    const end = new Date(sess.endTime).getTime()
    return s + (end - start) / 1000
  }, 0)
  const avgDuration = sessions.length > 0 ? totalDuration / sessions.length : 0

  return (
    <div className="stat-card">
      <div className="card-header">
        <div>
          <div className="card-subtitle">{tr.activity}</div>
          <div className="card-value success">{t(tr, 'sessionsCount', { count: sessions.length })}</div>
        </div>
        <div className="card-meta">
          <div>{tr.avg} {formatDuration(avgDuration)}</div>
          <div>{tr.total} {formatDuration(totalDuration)}</div>
        </div>
      </div>
      <div className="card-chart heatmap-wrap">
        <Heatmap sessions={sessions} noSessionsLabel={tr.noSessionsYet} />
      </div>
    </div>
  )
}

function ToolUsageCard({ sessions, tr }: { sessions: VibeSession[]; tr: ReturnType<typeof useTranslation> }): React.JSX.Element {
  const succeeded = sessions.reduce((s, sess) => s + (sess.stats.toolCallsSucceeded ?? 0), 0)
  const rejected = sessions.reduce((s, sess) => s + (sess.stats.toolCallsRejected ?? 0), 0)
  const failed = sessions.reduce((s, sess) => s + (sess.stats.toolCallsFailed ?? 0), 0)
  const total = sessions.reduce((s, sess) => s + (sess.stats.toolCallsAgreed ?? 0), 0)

  const donutData = [
    { label: tr.succeeded, value: succeeded, color: 'var(--color-success)' },
    { label: tr.rejectedLabel, value: rejected, color: 'var(--color-warning)' },
    { label: tr.failedLabel, value: failed, color: 'var(--color-danger)' }
  ].filter((d) => d.value > 0)

  return (
    <div className="stat-card tool-card">
      <div className="card-header">
        <div>
          <div className="card-subtitle">{tr.toolCalls}</div>
          <div className="card-value success">{total}</div>
        </div>
        <div className="card-meta">
          {rejected > 0 && <div className="warning-text">{rejected} {tr.rejected}</div>}
          {failed > 0 && <div className="danger-text">{failed} {tr.failed}</div>}
        </div>
      </div>
      <div className="tool-donut-center">
        {total > 0 ? (
          <DonutChart data={donutData} size={160} emptyLabel={tr.noToolCalls} totalLabel={tr.total} />
        ) : (
          <div className="chart-empty">{tr.noToolCalls}</div>
        )}
      </div>
      {donutData.length > 0 && (
        <div className="donut-legend" style={{ justifyContent: 'center' }}>
          {donutData.map((d) => (
            <div key={d.label} className="legend-item">
              <span className="legend-dot" style={{ background: d.color }} />
              <span className="legend-label">
                {d.label} ({d.value})
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ───── Session list ─────
function SessionList({ sessions, tr }: { sessions: VibeSession[]; tr: ReturnType<typeof useTranslation> }): React.JSX.Element {
  const [visibleCount, setVisibleCount] = useState(20)
  const loadMoreRef = useRef<HTMLDivElement>(null)

  const loadMore = useCallback(() => {
    setVisibleCount((prev) => Math.min(prev + 20, sessions.length))
  }, [sessions.length])

  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && visibleCount < sessions.length) {
          loadMore()
        }
      },
      { rootMargin: '100px' }
    )

    const sentinel = loadMoreRef.current
    if (sentinel) {
      observer.observe(sentinel)
    }

    return () => {
      if (sentinel) observer.unobserve(sentinel)
    }
  }, [loadMore, visibleCount, sessions.length])

  useEffect(() => {
    setVisibleCount(20)
  }, [sessions.length])

  return (
    <div className="session-list-card">
      <div className="section-label">{tr.recentSessions}</div>
      {sessions.length === 0 ? (
        <div className="empty-text">{tr.noSessionsFound}</div>
      ) : (
        <div className="session-rows">
          {sessions.slice(0, visibleCount).map((s) => {
            const start = new Date(s.startTime)
            const durationSecs = (new Date(s.endTime).getTime() - start.getTime()) / 1000
            const model = s.config?.activeModel ?? 'unknown'
            const project = s.environment?.workingDirectory?.split(/[/\\]/).pop() ?? '—'
            return (
              <div key={s.sessionId} className="session-row">
                <div className="session-row-main">
                  <span className="session-row-title">{s.title ?? tr.untitledSession}</span>
                  <div className="session-row-meta">
                    <span>{start.toLocaleDateString()}</span>
                    <span>{formatDuration(durationSecs)}</span>
                  </div>
                </div>
                <div className="session-row-tags">
                  <span className="tag primary">{project}</span>
                  <span className="tag accent">{model}</span>
                </div>
                {s.stats.sessionCachedTokens != null && s.stats.sessionPromptTokens > 0 && (
                  <div
                    className="session-row-cache"
                    title={"cached " + formatTokens(s.stats.sessionCachedTokens) + " / prompt " + formatTokens(s.stats.sessionPromptTokens)}
                  >
                    {String.fromCharCode(9889)} {Math.round((s.stats.sessionCachedTokens / s.stats.sessionPromptTokens) * 100)}%
                  </div>
                )}
                <div className="session-row-cost">${(s.stats.sessionCost ?? 0).toFixed(2)}</div>
              </div>
            )
          })}
          <div ref={loadMoreRef} className="load-more-sentinel">
            {visibleCount < sessions.length && (
              <span className="loading-indicator">{tr.loading}...</span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

// ───── Main dashboard ─────
export default function StatsView(): React.JSX.Element {
  const { config } = useStore()
  const tr = useTranslation(config.language)
  const [sessions, setSessions] = useState<VibeSession[]>([])
  const [loading, setLoading] = useState(true)
  const [timeRange, setTimeRange] = useState<TimeRange>('30d')
  const [customFrom, setCustomFrom] = useState<string>(() => {
    const d = new Date()
    d.setMonth(d.getMonth() - 1)
    return d.toISOString().slice(0, 10)
  })
  const [customTo, setCustomTo] = useState<string>(() => new Date().toISOString().slice(0, 10))

  const loadSessions = (): void => {
    setLoading(true)
    window.api.stats
      .vibeSessions()
      .then((data) => {
        setSessions(data as VibeSession[])
        setLoading(false)
      })
      .catch(() => setLoading(false))
  }

  useEffect(() => {
    loadSessions()
  }, [])

  const filtered = useMemo(() => {
    if (timeRange === 'all') return sessions
    if (timeRange === 'custom') {
      const from = new Date(customFrom)
      const to = new Date(customTo)
      to.setDate(to.getDate() + 1)
      return sessions.filter((s) => {
        const t = new Date(s.startTime)
        return t >= from && t < to
      })
    }
    const cutoff = new Date()
    const days = timeRange === '7d' ? 7 : timeRange === '30d' ? 30 : 90
    cutoff.setDate(cutoff.getDate() - days)
    return sessions.filter((s) => new Date(s.startTime) >= cutoff)
  }, [sessions, timeRange, customFrom, customTo])

  return (
    <div className="stats-view">
      {/* Toolbar */}
      <div className="stats-toolbar">
        <span className="stats-title">{tr.statistics}</span>
        <div className="time-range-btns">
          {(['7d', '30d', '90d', 'all', 'custom'] as TimeRange[]).map((r) => (
            <button
              key={r}
              className={`time-range-btn ${timeRange === r ? 'active' : ''}`}
              onClick={() => setTimeRange(r)}
            >
              {r === 'all' ? tr.allTime : r === 'custom' ? tr.custom : r}
            </button>
          ))}
        </div>
        {timeRange === 'custom' && (
          <div className="custom-range">
            <input
              type="date"
              className="date-input"
              value={customFrom}
              onChange={(e) => setCustomFrom(e.target.value)}
            />
            <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>–</span>
            <input
              type="date"
              className="date-input"
              value={customTo}
              onChange={(e) => setCustomTo(e.target.value)}
            />
          </div>
        )}
        <button className="refresh-btn" onClick={loadSessions} title={tr.refresh}>↻</button>
      </div>

      {loading ? (
        <div className="stats-loading">{tr.loading}</div>
      ) : (
        <div className="stats-scroll">
          <div className="cards-row">
            <CostCard sessions={filtered} tr={tr} />
            <TokenCard sessions={filtered} tr={tr} />
          </div>
          <div className="cards-row">
            <ActivityCard sessions={filtered} tr={tr} />
            <ToolUsageCard sessions={filtered} tr={tr} />
          </div>
          <SessionList sessions={filtered} tr={tr} />
        </div>
      )}
    </div>
  )
}
