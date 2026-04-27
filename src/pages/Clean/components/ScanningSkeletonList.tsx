import { DatabaseOutlined } from '@ant-design/icons'
import { MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'
import { CATEGORY_GROUPS, categoryIconMap } from '../clean.constants'
import { computeGroupStatusBySection } from '../scan-status'

export interface ScanningSkeletonListProps {
  /** 后端已完成扫描的 section 集合，用于判定各分组的扫描状态 */
  completedSections: Set<string>
}

/**
 * 扫描阶段骨架列表：铺开所有分类标题，按 section 完成情况
 * 实时显示「正在扫描垃圾... / 等待扫描」状态，避免扫描过程空白。
 *
 * 仅承担列表内容职责；头部与页面壳由 CleanLayout 提供。
 */
export default function ScanningSkeletonList({ completedSections }: ScanningSkeletonListProps) {
  return (
    <div className="pb-4">
      {CATEGORY_GROUPS.map((g) => {
        const status = computeGroupStatusBySection(completedSections, g.id)
        const icon = categoryIconMap[g.id] || <DatabaseOutlined />

        return (
          <div key={g.id} className="h-10 my-1 flex items-center gap-3">
            {icon}

            {/* 占位 checkbox（扫描中不可点） */}
            <MoleCheckbox checked={false} disabled />

            <span className="text-sm font-medium text-white shrink-0">{g.title}</span>

            {/* 副标题：状态文案 */}
            {status === 'scanning' ? (
              <span className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.warningYellow }}>
                正在扫描垃圾...
              </span>
            ) : (
              <span className="text-xs text-white/50">等待扫描</span>
            )}

            <div className="flex-1" />
          </div>
        )
      })}
    </div>
  )
}
