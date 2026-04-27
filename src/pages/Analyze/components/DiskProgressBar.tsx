import { memo, type CSSProperties } from 'react'

/**
 * DiskProgressBar — 磁盘用量渐变进度条
 *
 * 颜色通过 CSS 变量控制，支持 light/dark 主题切换：
 *   --progress-track       : 轨道背景色
 *   --progress-fill-start  : 渐变起始色（低用量）
 *   --progress-fill-end    : 渐变结束色（高用量）
 *
 * Dark  默认: track=rgba(0,0,0,0.3), fill=#4ade80 → #facc15 (green→yellow)
 * Light 建议: track=rgba(0,0,0,0.08), fill=#0d9488 → #ea580c (teal→orange)
 */
export const DiskProgressBar = memo(function DiskProgressBar({ percent, style }: { percent: number; style?: CSSProperties }) {
  return (
    <div
      className="h-1.5 rounded-full overflow-hidden w-full"
      style={{
        ...style,
        background: 'var(--progress-track, rgba(0, 0, 0, 0.3))',
      }}
    >
      <div
        className="h-full rounded-full transition-all duration-500 ease-out"
        style={{
          width: `${Math.max(percent, 1.5)}%`,
          background: `linear-gradient(90deg, var(--progress-fill-start, #4ade80), var(--progress-fill-end, #facc15))`,
        }}
      />
    </div>
  )
})
