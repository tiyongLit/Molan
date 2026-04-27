// 迷你趋势图，两种风格（对齐 Burrow MiniChart 分工规则）：
// - area：1px 描边 + 低透明度面积填充（内存/网络/风扇等连续量）
// - bars：底部对齐圆角柱（CPU/GPU 等离散利用率 %），真实像素宽度
//   驱动几何，纵轴锚定 0 诚实反映绝对值，空态画 25% 透明度底线占位。
// dual 模式：上下行双线双色（Burrow DualMiniChart 同款，网络卡用）。
// 纯 SVG 自绘，替代 ECharts，保证托盘气泡首帧轻量。

import { useContainerSize } from '@/hooks/useContainerSize'

interface SparklineProps {
  data: number[]
  color: string
  height?: number
  fillOpacity?: number
  /** 图表风格：'area' 折线（默认）/ 'bars' 柱状（Burrow CPU/GPU 同款） */
  style?: 'area' | 'bars'
  /** bars 专用：纵轴固定上限（如利用率传 100）——柱高与绝对值成正比，
   * 不被序列内偶发尖峰压扁；不传则自适应序列最大值（Burrow 默认行为） */
  domainMax?: number
  /** 双线模式（网络上行/下行）：设置后 data/color 作为下行线 */
  dual?: {
    up: number[]
    upColor: string
  }
  /** 镜像翻转（柠檬 NetworkPlotView.upsideDown=YES 同款）：
   * Y 轴反转，低值在顶部、高值在底部，面积填充朝顶部闭合。
   * 用于下载趋势图，与上传图形成蝴蝶对称效果。 */
  mirror?: boolean
}

/** Burrow MiniChart.samples 同款防御性上限：样本数封顶，避免无限序列驱动几何 */
const MAX_SAMPLES = 120

/** 把序列归一化成 SVG 路径（两线共享同一 min/max 域，可比）。
 * mirror=true 时 Y 轴翻转：低值在顶部、高值在底部（柠檬 upsideDown=YES 同款），
 * 面积填充朝 y=0（顶部）闭合，与正常图形成蝴蝶对称。 */
function buildPath(data: number[], min: number, max: number, mirror?: boolean): { line: string; area: string } {
  const W = 100
  const H = 28
  const span = max - min || 1
  const step = W / Math.max(1, data.length - 1)
  const pts = data.map((v, i) => {
    const x = i * step
    const y = mirror
      ? 3 + ((v - min) / span) * (H - 6)
      : H - 3 - ((v - min) / span) * (H - 6)
    return [x, y] as const
  })
  const line = pts.map(([x, y], i) => `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`).join(' ')
  const area = mirror
    ? `${line} L${W},0 L0,0 Z`
    : `${line} L${W},${H} L0,${H} Z`
  return { line, area }
}

/** 空态占位底线（Burrow MiniChart 同款）：25% 透明度，空卡片不显坏 */
function Baseline({ height, color }: { height: number; color: string }) {
  return (
    <svg viewBox="0 0 100 1" preserveAspectRatio="none" className="block w-full" style={{ height }} aria-hidden>
      <line x1={0} y1={0.5} x2={100} y2={0.5} stroke={color} strokeOpacity={0.25} strokeWidth={1} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

/** 单根圆角柱的 SVG 路径（r = 圆角半径，从左下角顺时针闭合） */
function roundedBarPath(x: number, y: number, w: number, h: number, r: number): string {
  const rr = Math.min(r, w / 2, h / 2)
  return [
    `M${x.toFixed(1)},${(y + h).toFixed(1)}`,
    `L${x.toFixed(1)},${(y + rr).toFixed(1)}`,
    `Q${x.toFixed(1)},${y.toFixed(1)} ${(x + rr).toFixed(1)},${y.toFixed(1)}`,
    `L${(x + w - rr).toFixed(1)},${y.toFixed(1)}`,
    `Q${(x + w).toFixed(1)},${y.toFixed(1)} ${(x + w).toFixed(1)},${(y + rr).toFixed(1)}`,
    `L${(x + w).toFixed(1)},${(y + h).toFixed(1)}`,
    'Z'
  ].join(' ')
}

/** 柱状迷你图（Burrow MiniChart.bars 一比一 + 绝对纵轴增强）：单个 <path>
 * 拼所有底部对齐圆角柱，slot 整数像素对齐（间隙均匀不抗锯齿模糊），
 * 柱宽 62% slot、纵轴锚定 0，0.85 透明度纯色填充。
 * domainMax 设置后柱高 = 值/domainMax × 高（绝对比例，1% vs 10% 严格 1:10） */
function Bars({ data, color, height, width, domainMax }: { data: number[]; color: string; height: number; width: number; domainMax?: number }) {
  if (data.length < 2 || width <= 0) return <Baseline height={height} color={color} />
  const vals = data.slice(-MAX_SAMPLES)
  const n = Math.max(vals.length, 1)
  const slot = width / n
  // 纵轴：固定域（绝对比例，尖峰不压扁低值柱）或自适应序列最大值（Burrow 同款）；
  // 全平序列 hi 垫到 1，柱子保留最小高度
  const hi = domainMax ?? Math.max(Math.max(...vals), 0.0001)
  let d = ''
  for (let i = 0; i < n; i++) {
    const bh = Math.max(1.5, (Math.min(vals[i], hi) / hi) * height)
    // slot 边界取整到设备像素：柱子间距严格均匀，消除亚像素舍入导致的
    // 间隙不均/边缘发虚（对齐 Burrow Core Graphics 的像素对齐渲染）
    const x0 = Math.round(i * slot)
    const x1 = Math.round((i + 1) * slot)
    const sw = Math.max(x1 - x0, 1)
    const barW = Math.max(1.5, sw * 0.62)
    const x = x0 + (sw - barW) / 2
    d += roundedBarPath(x, height - bh, barW, bh, 1) + ' '
  }
  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} className="block" aria-hidden>
      <path d={d} fill={color} fillOpacity={0.85} />
    </svg>
  )
}

/** bars 模式容器：ResizeObserver 测真实宽度驱动柱子几何（柱宽恒定不随拉伸变形） */
function BarSparkline({ data, color, height, domainMax }: { data: number[]; color: string; height: number; domainMax?: number }) {
  const { ref, width } = useContainerSize<HTMLDivElement>()
  return (
    <div ref={ref} className="w-full">
      <Bars data={data} color={color} height={height} width={width} domainMax={domainMax} />
    </div>
  )
}

export function Sparkline({ data, color, height = 14, fillOpacity = 0.35, style = 'area', domainMax, dual, mirror }: SparklineProps) {
  if (style === 'bars') return <BarSparkline data={data} color={color} height={height} domainMax={domainMax} />

  const up = dual?.up
  const hasDual = Boolean(up && up.length >= 2)
  // 空态占位底线（Burrow 同款）：不再 return null，图表区高度保持稳定
  if (data.length < 2 && !hasDual) return <Baseline height={height} color={color} />

  const vals = data.slice(-MAX_SAMPLES)
  const upVals = hasDual ? (up as number[]).slice(-MAX_SAMPLES) : undefined
  const all = upVals ? [...vals, ...upVals] : vals
  const min = Math.min(...all)
  const max = Math.max(...all)
  const down = vals.length >= 2 ? buildPath(vals, min, max, mirror) : null
  const upPath = upVals ? buildPath(upVals, min, max, mirror) : null

  return (
    <svg
      viewBox="0 0 100 28"
      preserveAspectRatio="none"
      className="block w-full"
      style={{ height }}
      aria-hidden
    >
      {down && (
        <>
          <path d={down.area} fill={color} opacity={fillOpacity} />
          <path
            d={down.line}
            fill="none"
            stroke={color}
            strokeWidth={2}
            strokeLinecap="round"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </>
      )}
      {upPath && dual && (
        <path
          d={upPath.line}
          fill="none"
          stroke={dual.upColor}
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  )
}
