import { AnimatePresence, motion } from 'framer-motion'
import { DatabaseOutlined } from '@ant-design/icons'
import { formatSize } from '@/utils/format'
import { ExpandChevron, MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'
import { useI18n } from '@/i18n'
import { categoryIconMap, type CleanGroupData } from '../clean.constants'
import { selKey } from '../scan-status'
import CleanItemRow from './CleanItemRow'
import type { MoleCleanItem } from '@/types/mole'

interface CategoryRowProps {
  group: CleanGroupData
  isExpanded: boolean
  isIdle: boolean
  isCleaning: boolean
  /** 渐进式扫描中：分类已到达但整轮扫描未结束，禁用勾选/折叠交互，避免半成品选择 */
  isScanning?: boolean
  cleanedItemKeys: Set<string>
  selectedItemIds: Set<string>
  onToggleCategory: (groupId: string, items: MoleCleanItem[]) => void
  onToggleGroup: (group: CleanGroupData) => void
  onToggleItem: (categoryId: string, itemId: string) => void
  onReveal: (path: string) => void
}

/**
 * 分组行 + 展开的子项列表（lemon-cleaner CategoryCellView 风格）。
 * 子项行由 CleanItemRow（memo）渲染，清理动画阶段仅受影响行重渲染。
 */
export default function CategoryRow({
  group,
  isExpanded,
  isIdle,
  isCleaning,
  isScanning = false,
  cleanedItemKeys,
  selectedItemIds,
  onToggleCategory,
  onToggleGroup,
  onToggleItem,
  onReveal,
}: CategoryRowProps) {
  const { t } = useI18n()
  const allSelected = group.itemCount > 0 && group.selectedCount === group.itemCount
  const partialSelected = group.selectedCount > 0 && group.selectedCount < group.itemCount
  const isClean = group.itemCount === 0 || group.totalSize === 0
  const icon = categoryIconMap[group.id] || <DatabaseOutlined />

  const visibleItems = isCleaning
    ? group.items.filter(item => !cleanedItemKeys.has(selKey(item.categoryId, item.id)))
    : group.items

  return (
    <div>
      {/* ── 分组标题行 ── */}
      <div
        className="cursor-pointer hover:bg-white/[0.06] transition-colors h-10 my-1 flex items-center gap-3 px-[24px]"
        onClick={() => !isCleaning && !isScanning && onToggleCategory(group.id, group.items)}
      >
        {icon}

        <MoleCheckbox
          checked={allSelected}
          partial={partialSelected}
          disabled={isClean || isCleaning || isScanning}
          onClick={(e) => {
            e.stopPropagation()
            if (!isClean && !isCleaning && !isScanning) onToggleGroup(group)
          }}
        />

        <span className="text-sm font-medium text-white shrink-0">{t(group.titleKey)}</span>

        {!isClean && (
          <span className="text-xs text-white/50">
            {t('clean.category.totalWithSelected', { size: formatSize(group.totalSize) })}
            <span className="ml-1 font-medium" style={{ color: SEMANTIC_COLORS.warningYellow }}>
              {formatSize(group.selectedSize)}
            </span>
          </span>
        )}

        <div className="flex-1" />

        {isIdle ? null : isClean ? (
          <span className="text-xs font-medium shrink-0" style={{ color: SEMANTIC_COLORS.successGreen }}>
            {t('clean.common.clean')}
          </span>
        ) : (
          <span className="text-xs font-medium text-white shrink-0">
            {formatSize(group.totalSize)}
          </span>
        )}

        {!isIdle && <ExpandChevron expanded={isExpanded} />}
      </div>

      {/* ── 展开的子项（CSS grid 过渡，避免 layout 测量） ── */}
      {!isClean && (
        <div
          className={`grid overflow-hidden transition-[grid-template-rows,opacity] duration-250 ease-[cubic-bezier(0.4,0,0.2,1)] ${isExpanded ? 'opacity-100' : 'opacity-0'
            }`}
          style={{ gridTemplateRows: isExpanded ? '1fr' : '0fr' }}
        >
          <div className="min-h-0">
            <AnimatePresence>
              {visibleItems.map((item) => {
                const itemKey = selKey(item.categoryId, item.id)
                return (
                  <motion.div
                    key={itemKey}
                    exit={{ opacity: 0, height: 0 }}
                    transition={{ duration: 0.35, ease: 'easeInOut' }}
                    className="overflow-hidden"
                  >
                    <CleanItemRow
                      item={item}
                      isSelected={selectedItemIds.has(itemKey)}
                      isCleaning={isCleaning || isScanning}
                      onToggleItem={onToggleItem}
                      onReveal={onReveal}
                    />
                  </motion.div>
                )
              })}
            </AnimatePresence>
          </div>
        </div>
      )}
    </div>
  )
}
