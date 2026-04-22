import { useMemo } from 'react'
import { Wifi } from 'lucide-react'
import { Sparkline } from './Sparkline'
import { dashTheme } from './theme'
import { useI18n } from '@/i18n'

/** 柠檬格式规则：< 1000 KB/s → KB/s（整数），≥ 1000 KB/s → MB/s（一位小数） */
function fmtNetRate(mbs: number): { v: string; u: string } {
  const kb = mbs * 1024
  if (kb >= 1000) return { v: (kb / 1024).toFixed(1), u: 'MB/s' }
  return { v: String(Math.max(0, Math.round(kb))), u: 'KB/s' }
}

export function NetworkCard({
  downMBs,
  upMBs,
  downHist,
  upHist
}: {
  /** 下行速率 MB/s（原始值，组件内部格式化） */
  downMBs: number
  /** 上行速率 MB/s（原始值，组件内部格式化） */
  upMBs: number
  /** 下行历史序列（MB/s） */
  downHist: number[]
  /** 上行历史序列（MB/s） */
  upHist: number[]
}) {
  const { t } = useI18n()
  const down = fmtNetRate(downMBs)
  const up = fmtNetRate(upMBs)

  // 柠檬 displayMax：两个 Sparkline 共享同一纵轴上限（视觉可对比上下行量级差异）
  const displayMax = useMemo(() => {
    const all = [...downHist, ...upHist]
    // 柠檬 NetworkMinMaxValue = 50*1024 B/s → 0.05 MB/s：空闲时平底线，微小流量不顶满格
    return all.length ? Math.max(...all, 0.05) : 0.05
  }, [downHist, upHist])

  return (
    <div
      className="flex min-w-0 flex-col gap-2 overflow-hidden rounded-[14px] px-3 pb-3 pt-2.5"
      style={{ background: dashTheme.card, border: `1px solid ${dashTheme.cardBorder}`, height: 140 }}
    >
      {/* Header：网络标题 + 实时速率（对齐柠檬网络测速样式） */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1">
          <span style={{ color: dashTheme.textTertiary }}>
            <Wifi size={12} />
          </span>
          <span className="shrink-0 text-[11px] font-medium" style={{ color: dashTheme.textTertiary }}>
            {t('dashboard.network.title')}
          </span>
        </div>
        <div className="flex items-center gap-2 text-[11px] font-semibold tabular-nums">
          <span style={{ color: '#4ade80' }}>
            ↓{down.v}
            <span className="ml-0.5 text-[9px] font-medium" style={{ color: dashTheme.textTertiary }}>
              {down.u}
            </span>
          </span>
          <span className="mx-0.5" style={{ color: dashTheme.textTertiary }}>·</span>
          <span style={{ color: '#60a5fa' }}>
            ↑{up.v}
            <span className="ml-0.5 text-[9px] font-medium" style={{ color: dashTheme.textTertiary }}>
              {up.u}
            </span>
          </span>
        </div>
      </div>

      {/* 上传趋势（蓝色，正常朝向 — 高峰朝上）
          柠檬 butterfly 语义：两个图表的半透明填充在中间位置自然衔接。
          纯 SVG 自绘（替代 ECharts），零动画开销，即时渲染对齐柠檬 Core Graphics 语义。 */}
      <div className="flex-1">
        <Sparkline
          data={upHist}
          color="#60a5fa"
          height={45}
          domainMax={displayMax}
        />
      </div>

      {/* 下载趋势（绿色，mirror — 高峰朝下，与上传形成蝴蝶对称）
          marginTop: -8px 使两个图表的基线在中间重合（45px × 2 - 8px overlap = 82px 总高） */}
      <div style={{ marginTop: -8 }}>
        <Sparkline
          data={downHist}
          color="#4ade80"
          height={45}
          domainMax={displayMax}
          mirror
        />
      </div>
    </div>
  )
}
