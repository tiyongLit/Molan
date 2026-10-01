import { DatabaseOutlined } from '@ant-design/icons'
import { MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'
import { useI18n } from '@/i18n'
import { CATEGORY_GROUPS, categoryIconMap, type CleanGroupData } from '../clean.constants'
import { computeGroupStatusBySection, type ScanGroupStatus } from '../scan-status'

/**
 * 单个分组的扫描占位行。
 * 渐进式扫描下，index.tsx 逐组混合渲染：已到达条目的分组走 CategoryRow（真列表），
 * 尚未到达的分组走本占位行，显示「正在扫描垃圾... / 等待扫描」。
 * px-[24px] 与 CategoryRow 标题行对齐（外层 ScanPageLayout 已 pl-24，合计 48px）。
 */
export function ScanningGroupRow({
  group,
  status,
}: {
  group: Pick<CleanGroupData, 'id' | 'titleKey'>
  status: ScanGroupStatus
}) {
  const { t } = useI18n()
  const icon = categoryIconMap[group.id] || <DatabaseOutlined />
  return (
    <div className="h-10 my-1 flex items-center gap-3 px-[24px]">
      {icon}

      {/* 占位 checkbox（扫描中不可点） */}
      <MoleCheckbox checked={false} disabled />

      <span className="text-sm font-medium text-white shrink-0">{t(group.titleKey)}</span>

      {/* 副标题：状态文案 */}
      {status === 'scanning' ? (
        <span className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.warningYellow }}>
          {t('clean.status.scanningTrash')}
        </span>
      ) : (
        <span className="text-xs text-white/50">{t('clean.status.waiting')}</span>
      )}

      <div className="flex-1" />
    </div>
  )
}

export interface ScanningSkeletonListProps {
  /** 后端已完成扫描的 section 集合，用于判定各分组的扫描状态 */
  completedSections: Set<string>
}

/**
 * 扫描阶段骨架列表（全部占位）：铺开所有分类标题，按 section 完成情况显示状态。
 * 渐进式扫描下，尚无任何分类到达时作为整体占位；有分类到达后 index.tsx 改走逐组混合渲染。
 *
 * 仅承担列表内容职责；头部与页面壳由 ScanPageLayout 提供。
 */
export default function ScanningSkeletonList({ completedSections }: ScanningSkeletonListProps) {
  return (
    <div className="pb-4">
      {CATEGORY_GROUPS.map((g) => (
        <ScanningGroupRow
          key={g.id}
          group={g}
          status={computeGroupStatusBySection(completedSections, g.id)}
        />
      ))}
    </div>
  )
}
