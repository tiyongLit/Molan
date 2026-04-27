import { memo, useCallback, useMemo } from 'react'
import { FolderOpen } from 'lucide-react'
import { formatSize } from '@/utils/format'
import { SIZE_COLOR_CLASS } from '../utils/sizeColor'
import { entrySubtitle } from '../utils/fileType'
import type { MoleAnalyzeEntry } from '@/types/mole'

interface EntryRowProps {
  idx: number
  entry: MoleAnalyzeEntry
  icon: string
  isChecked: boolean
  isProtected?: boolean
  isFocused: boolean
  /** Top 20 模式：显示完整路径 + Finder 按钮 */
  showPath?: boolean
  /** 在 Finder 中显示（Top 20 行右侧按钮回调） */
  onReveal?: (path: string) => void
}

/**
 * 单行条目渲染 — 纯展示组件。
 *
 * **设计原则（对标 lemon-cleaner LMSpaceTableRowView）**：
 *   - 磁盘空间分析是「绝对大小对比」场景，没有「独立上限」语义；
 *     Treemap 矩形面积本身已反映相对占比，行内进度条是冗余的视觉噪声。
 *   - 行内承载 4 列信息：checkbox / 图标 / 名称+副标题 / 大小。
 *   - 副标题（差异化文案，详见 utils/fileType.ts 的回退层级）：
 *     · Top 20（文件）→ 完整路径（现状保留）
 *     · symlink → "符号链接"（后端显式标记）
 *     · 目录 + bundle 后缀（.app/.framework…）→ "应用程序"/"框架"…
 *     · 普通目录 → "X 项"（柠檬式单数字，后端扫描时统计）
 *     · 文件 + 扩展名命中 → "ZIP 归档"/"PNG 图像"…（constants.rs 单一事实来源表）
 *     · 未知类型 → 空白（对齐柠檬"未知类型不显示"）
 *
 * 行交互（选中/焦点/钻入）由父组件通过 data-idx / data-protected 事件委托统一处理；
 * 仅 Finder 定位按钮通过 onReveal 回调上报（需 stopPropagation 避免触发行事件）。
 */
export const EntryRow = memo(function EntryRow({
  idx,
  entry,
  icon,
  isChecked,
  isProtected = false,
  isFocused,
  showPath = false,
  onReveal
}: EntryRowProps) {
  // 大小颜色按「相对当前目录总容量」分级，与 LocationSelector 容量条一致
  const sizeColor = SIZE_COLOR_CLASS.gray

  // 副标题：统一槽位，按行类型与模式决定文案（详见 utils/fileType.ts）
  const subtitle = useMemo(() => entrySubtitle(entry, showPath), [entry, showPath])

  const hints: string[] = []
  if (entry.is_dir && entry.cleanable) {
    hints.push('🧹')
  }
  if (entry.is_bundle_leaf) {
    hints.push('📦')
  }
  if (isProtected) {
    hints.push('🔒')
  }

  // Finder 按钮：阻断冒泡，避免触发行的 click（焦点）与 dblclick（钻入）
  const handleReveal = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      onReveal?.(entry.path)
    },
    [entry.path, onReveal]
  )

  return (
    <div
      data-idx={idx}
      data-protected={isProtected}
      className={`flex items-center gap-2.5 px-3 py-1.5 rounded-lg mx-1 transition-colors duration-150 select-none ${
        isFocused
          ? 'bg-white/[0.10] shadow-[inset_2px_0_0_0_rgba(255,255,255,0.65)]'
          : 'hover:bg-black/[0.25]'
      }`}
      style={{
        cursor: isProtected ? 'default' : 'pointer',
        opacity: isProtected ? 0.55 : 1
      }}
    >
      <input
        type="checkbox"
        checked={isChecked}
        readOnly
        disabled={isProtected}
        className="entry-checkbox-xs shrink-0"
      />

      {/* 图标（对齐启动项：28px 容器，原生系统图标优先） */}
      <div className="shrink-0 flex items-center justify-center" style={{ width: 28, height: 28 }}>
        {icon.startsWith('data:') ? (
          <img src={icon} alt="" className="w-[28px] h-[28px] object-contain" />
        ) : (
          <span className="text-xs leading-none text-center">{icon}</span>
        )}
      </div>

      {/* 名称 + 副标题（对标 lemon-cleaner：13px 名称 + 10px mono 副标题） */}
      <div className="flex-1 min-w-0 flex flex-col gap-0.5">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-[13px] font-medium truncate text-[var(--text-primary)]">
            {entry.name}
          </span>
          {hints.length > 0 && (
            <span className="text-[10px] text-white/40 shrink-0 whitespace-nowrap">
              {hints.join(' ')}
            </span>
          )}
        </div>
        {subtitle && (
          <span className="text-[10px] text-white/60 truncate block font-mono" title={subtitle}>
            {subtitle}
          </span>
        )}
      </div>

      {/* 大小：单列、统一右对齐、12px 等宽数字（lemon-cleaner 颜色 #989A9E） */}
      <span
        className={`text-xs font-mono tabular-nums shrink-0 ${sizeColor}`}
        title={formatSize(entry.size)}
      >
        {formatSize(entry.size)}
      </span>

      {/* 在 Finder 中显示（对齐启动项列表右侧操作按钮，仅 Top 20 模式） */}
      {showPath && (
        <button
          onClick={handleReveal}
          title="在 Finder 中显示"
          className="text-white/60 hover:text-[var(--text-primary)] shrink-0"
        >
          <FolderOpen size={14} />
        </button>
      )}
    </div>
  )
})
