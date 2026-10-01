// 迷你趋势图（area 模式）：1px 描边 + 低透明度面积填充（网络等连续量）。
// mirror 模式：Y 轴反转，低值在顶部、高值在底部（柠檬 NetworkPlotView.upsideDown=YES），
// 面积填充朝 y=0（顶部）闭合，与正常图形成蝴蝶对称。
// domainMax：纵轴锚定 [0, domainMax]，尖峰不会压扁后续小值。
// 纯 SVG 自绘，替代 ECharts，保证托盘气泡首帧轻量。

interface SparklineProps {
  data: number[]
  color: string
  height?: number
  fillOpacity?: number
  /** 纵轴固定上限（不传则自适应序列最大值） */
  domainMax?: number
  /** 镜像翻转（柠檬 NetworkPlotView.upsideDown=YES 同款）：
   * Y 轴反转，低值在顶部、高值在底部，面积填充朝顶部闭合。
   * 用于下载趋势图，与上传图形成蝴蝶对称效果。 */
  mirror?: boolean
}

/** 防御性上限：样本数封顶，避免无限序列驱动几何 */
const MAX_SAMPLES = 120

/** 把序列归一化成 SVG 路径。
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

/** 空态占位底线：25% 透明度，空卡片不显坏 */
function Baseline({ height, color }: { height: number; color: string }) {
  return (
    <svg viewBox="0 0 100 1" preserveAspectRatio="none" className="block w-full" style={{ height }} aria-hidden>
      <line x1={0} y1={0.5} x2={100} y2={0.5} stroke={color} strokeOpacity={0.25} strokeWidth={1} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

export function Sparkline({ data, color, height = 14, fillOpacity = 0.35, domainMax, mirror }: SparklineProps) {
  // 空态占位底线：不再 return null，图表区高度保持稳定
  if (data.length < 2) return <Baseline height={height} color={color} />

  const vals = data.slice(-MAX_SAMPLES)
  // domainMax 生效：纵轴锚定 [0, domainMax]，尖峰不会压扁后续小值（area 模式核心修复）。
  // 不传则保持自适应行为（默认），向后兼容。
  const min = domainMax != null ? 0 : Math.min(...vals)
  const max = domainMax != null ? domainMax : Math.max(...vals)
  const path = buildPath(vals, min, max, mirror)

  return (
    <svg
      viewBox="0 0 100 28"
      preserveAspectRatio="none"
      className="block w-full"
      style={{ height }}
      aria-hidden
    >
      <path d={path.area} fill={color} opacity={fillOpacity} />
      <path
        d={path.line}
        fill="none"
        stroke={color}
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  )
}
