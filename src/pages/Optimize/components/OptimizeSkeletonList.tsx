import { SEMANTIC_COLORS } from '@/constants/theme'
import { TASK_GROUPS, groupIconMap } from '../optimize.constants'

export interface OptimizeSkeletonListProps {
  /** 当前已分析到的分组下标（用于判定各分组的 done / running / pending 状态） */
  analyzeGroupIdx: number
}

/**
 * 分析阶段骨架列表：铺开所有分组标题，按进度实时显示
 * 「正在分析… / 等待分析 / ✓」状态，避免分析过程空白。
 *
 * 仅承担列表内容职责；头部与页面壳由 ScanPageLayout 提供。
 */
export default function OptimizeSkeletonList({ analyzeGroupIdx }: OptimizeSkeletonListProps) {
  return (
    <div>
      {TASK_GROUPS.map((g, i) => {
        const state = i < analyzeGroupIdx ? 'done' : i === analyzeGroupIdx ? 'running' : 'pending'
        const icon = groupIconMap[g.id]

        return (
          <div key={g.id} className="h-10 my-1 flex items-center px-[52px] gap-3">
            {icon}

            {/* 占位 checkbox（分析中不可点） */}
            <span
              className="w-3.5 h-3.5 rounded-[2px] border-[1.5px] shrink-0"
              style={{ borderColor: 'rgba(255,255,255,0.15)', backgroundColor: 'rgba(255,255,255,0.05)' }}
            />

            <span className="text-sm font-medium text-white shrink-0">{g.title}</span>

            {/* 副标题：状态文案 */}
            {state === 'running' ? (
              <span className="text-xs font-medium flex items-center gap-1.5" style={{ color: 'rgb(255, 190, 70)' }}>
                <span className="optimize-group-pulse" />
                正在分析…
              </span>
            ) : state === 'done' ? (
              <span className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.successGreen }}>✓</span>
            ) : (
              <span className="text-xs text-white/50">等待分析</span>
            )}

            <div className="flex-1" />
            <span className="text-xs text-white/30">{g.actionIds.length} 项</span>
          </div>
        )
      })}
    </div>
  )
}
