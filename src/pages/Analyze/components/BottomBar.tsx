import { MoleButton } from '@/components/ui'
import { DeleteOutlined } from '@ant-design/icons'
import { useAnalyze } from '../contexts/AnalyzeContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { formatSize } from '@/utils/format'
import { useMemo } from 'react'
import { MarqueeText } from '@/components/ui/MarqueeText'

/**
 * 底部操作栏（单行布局）：
 *   左（flex-1，超宽跑马灯滚动）：当前层级统计 | 已选/过滤/文件统计
 *   右（shrink-0，固定不受左侧挤压）：移到废纸篓按钮
 */
export function BottomBar() {
  const sel = useAnalyzeSelection()
  const ctx = useAnalyze()
  const { checkedStats, hasSelection, showTop20, activeData } = sel
  const { trashSelected, trashing, filtering, filterQuery, entries, totalFiles } = ctx

  // 当前层级文件/文件夹计数
  const fileCount = useMemo(() => entries.filter((e) => !e.is_dir).length, [entries])
  const dirCount = useMemo(() => entries.filter((e) => e.is_dir).length, [entries])

  return (
    <>
      {/* hairline 分隔线（对齐 Uninstall：底栏无背景，直接浮在渐变上） */}
      <div className="shrink-0 h-px bg-white/[0.10]" />
      <div className="flex items-center justify-between h-[40px] shrink-0 px-[24px] gap-3">
        {/* 左：合并统计（原左侧列表底部统计 + 底栏三态信息），超宽跑马灯滚动 */}
        <div className="flex-1 min-w-0">
          <MarqueeText className="text-xs">
            {/* 段1：当前层级统计 */}
            <span className="text-white/40">
              {formatSize(activeData.total)} · {activeData.items.length} 项
            </span>
            {/* 竖分隔符 */}
            <span className="mx-2.5 w-px h-3 bg-white/[0.15] shrink-0" />
            {/* 段2：选择 / 过滤 / 文件统计三态 */}
            {hasSelection ? (
              <span className="text-white/70">
                已选 <span className="text-[#FFBE46] font-medium">{checkedStats.count}</span>{' '}
                项 ·{' '}
                <span className="text-[#FFBE46] font-medium font-mono">
                  {formatSize(checkedStats.size)}
                </span>
              </span>
            ) : filtering || filterQuery ? (
              <span className="text-white/70">
                <span className="text-cyan-400">/</span> Filter
                {filterQuery ? ` · "${filterQuery}"` : ''}
                {' — '}Esc clear
              </span>
            ) : (
              <span className="text-white/40">
                {showTop20
                  ? `Top 20 大文件 · ${totalFiles > 0 ? `总计 ${totalFiles.toLocaleString()} 个文件` : ''}`
                  : `${dirCount} 文件夹 · ${fileCount} 文件${totalFiles > 0 ? ` · 总计 ${totalFiles.toLocaleString()} 个文件` : ''}`}
              </span>
            )}
          </MarqueeText>
        </div>

        {/* 右：废纸篓按钮（shrink-0 固定，不受左侧文本长度影响） */}
        <MoleButton
          size="small"
          disabled={!hasSelection || trashing}
          loading={trashing}
          icon={<DeleteOutlined />}
          style={{ fontSize: 12, borderRadius: 3 }}
          className="analyze-trash-btn shrink-0"
          onClick={trashSelected}
        >
          移到废纸篓{hasSelection ? ` (${checkedStats.count})` : ''}
        </MoleButton>
      </div>
    </>
  )
}
