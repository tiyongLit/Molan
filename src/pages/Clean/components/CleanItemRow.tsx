import { memo } from 'react'
import { ConfigProvider } from 'antd'
import { FolderOpenOutlined } from '@ant-design/icons'
import { formatSize } from '@/utils/format'
import { MoleButton, MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'
import type { MoleCleanItem } from '@/types/mole'

interface CleanItemRowProps {
  item: MoleCleanItem
  isSelected: boolean
  isCleaning: boolean
  onToggleItem: (categoryId: string, itemId: string) => void
  onReveal: (path: string) => void
}

/**
 * 子项行（memo 化）：清理阶段 cleanedItemKeys 每 400ms 变化时，
 * 只有被弹出动画的行触发重渲染，其余行跳过。
 * 注意 onToggleItem / onReveal 必须由调用方保证引用稳定，memo 才生效。
 */
const CleanItemRow = memo(function CleanItemRow({
  item,
  isSelected,
  isCleaning,
  onToggleItem,
  onReveal,
}: CleanItemRowProps) {
  const isEmpty = item.size === 0 && item.file_count === 0
  const displayPath = item.path
    ? item.path.replace(/^\/Users\/[^/]+/, '~')
    : item.id

  const toggle = () => {
    if (!isEmpty && !isCleaning) onToggleItem(item.categoryId, item.id)
  }

  return (
    <div
      className={`group cursor-pointer hover:bg-white/[0.06] transition-colors h-8 flex items-center pl-[71px] pr-[24px] gap-2 ${isEmpty ? 'opacity-50 cursor-not-allowed' : ''}`}
      onClick={toggle}
    >
      <MoleCheckbox
        checked={isSelected}
        disabled={isEmpty}
        className={isEmpty ? 'cursor-not-allowed' : ''}
        onClick={(e) => {
          e.stopPropagation()
          toggle()
        }}
      />

      <div className="flex items-center gap-1 flex-1 min-w-0">
        <span className="text-xs text-white truncate leading-4">{displayPath}</span>
        {item.real_path && (
          <ConfigProvider theme={{
            components: {
              Button: {
                defaultColor: 'rgba(255,255,255,0.35)',           // 柔雾灰（首推）
                defaultHoverColor: '#ffffff',      // 悬停纯白
                defaultActiveColor: 'rgba(255,255,255,0.12)',     // 点击冷灰（与常态形成明显色差）
              }
            }
          }}>
            <MoleButton
              size="small"
              color='default'
              variant='link'
              className="opacity-0 group-hover:opacity-60 hover:!opacity-100 transition-opacity"
              icon={<FolderOpenOutlined style={{ fontSize: 12 }} />}
              onClick={(e) => {
                e.stopPropagation()
                onReveal(item.real_path!)
              }}
            >
            </MoleButton>
          </ConfigProvider>
        )}
      </div>

      <span className="shrink-0 flex items-center gap-2">
        {item.file_count > 0 && (
          <span className="text-xs font-medium text-white/50">{item.file_count} 个文件</span>
        )}
        {isEmpty ? (
          <span className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.successGreen }}>很干净</span>
        ) : item.cautious ? (
          <span className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.cautiousOrange }}>
            共 {item.size_human || formatSize(item.size)}，谨慎清理
          </span>
        ) : (
          <span className="text-xs font-medium text-white/50">
            共 {item.size_human || formatSize(item.size)}，建议清理
          </span>
        )}
      </span>
    </div>
  )
})

export default CleanItemRow
