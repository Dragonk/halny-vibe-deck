import React from 'react'

const COL_HEIGHTS = [2, 4, 6, 3, 6, 4, 2]
const LEVEL_COLORS = ['#E85D04', '#E85D04', '#FF8205', '#FF8205', '#FFB020', '#FFD800']
const UNIT = 60
const GAP = 10
const X0 = 272
const BASE = 792

interface Block {
  x: number
  y: number
  fill: string
}

const BLOCKS: Block[] = []
COL_HEIGHTS.forEach((h, c) => {
  for (let k = 0; k < h; k++) {
    BLOCKS.push({ x: X0 + c * (UNIT + GAP), y: BASE - (k + 1) * UNIT - k * GAP, fill: LEVEL_COLORS[k] })
  }
})

export default function BrandIcon({ size = 72, className }: { size?: number; className?: string }): React.JSX.Element {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width={size} height={size} viewBox="0 0 1024 1024" className={className} role="img" aria-label="Halny Vibe Deck">
      <rect x={0} y={0} width={1024} height={1024} rx={220} fill="#10131A" />
      <g shapeRendering="crispEdges">
        {BLOCKS.map((b, i) => (
          <rect key={i} x={b.x} y={b.y} width={UNIT} height={UNIT} fill={b.fill} />
        ))}
      </g>
      <path d="M 258 330 Q 512 262 766 290" fill="none" stroke="#F2F5FA" strokeWidth={26} strokeLinecap="round" />
      <path d="M 272 358 Q 512 298 750 318" fill="none" stroke="#FFD800" strokeWidth={11} strokeLinecap="round" />
    </svg>
  )
}
