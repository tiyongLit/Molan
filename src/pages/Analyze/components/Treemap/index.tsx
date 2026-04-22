import { useMemo, useCallback } from 'react'
import { RectBlock } from './RectBlock'
import { computeTreemapRects } from './computeTreemap'
import { useContainerSize } from '@/hooks/useContainerSize'
import { useI18n } from '@/i18n'
import { ScanOverlay } from './ScanOverlay'
import type { TreemapItem } from '../../typings'
import type { ScanProgress } from '../../hooks/useAnalyzeData'

interface TreemapProps {
  items: TreemapItem[]
  onDrillIn: (item: TreemapItem) => void
  scanning: boolean
  scanProgress: ScanProgress | null
  /** 扫描取消中（传给 ScanOverlay：按钮禁用 + 「正在取消...」） */
  cancelling?: boolean
  /** 取消扫描回调；缺省时浮层不渲染取消按钮 */
  onCancelScan?: () => void
  /** 勾选集合（path 对齐）— treemapItems 经上限截断后索引与 activeData 不再对齐，统一按 path 匹配 */
  checkedPaths: Set<string>
  /** 当前焦点条目路径（来自左侧列表 focusedIdx）— 键盘 ↑↓ 时同步高亮对应方格 */
  focusedPath: string | null
  /** 单击方格回调 — 同步左侧列表焦点 */
  onFocusPath: (path: string) => void
}

/**
 * Treemap 可视化 — 事件委托架构。
 *
 * **交互委托**：click / dblclick 统一在容器层处理，通过 `data-path` 定位目标条目。
 * RectBlock 只接收纯数据 props（item / isSelected / isChecked），不传任何回调闭包，
 * 确保 memo 真正生效 —— 只有数据变化的方格才重渲染。
 *
 * **右键菜单 / 悬停气泡**：由全局单例 AnalyzeContextMenu / AnalyzeHoverCard 接管，
 * RectBlock 不再包裹 antd Dropdown / Popover，组件树深度大幅降低。
 *
 * **hover 视觉**：纯 CSS `:hover` 实现（RectBlock 内的 transition），不依赖 JS state。
 */
export function Treemap({
  items,
  onDrillIn,
  scanning,
  scanProgress,
  cancelling,
  onCancelScan,
  checkedPaths,
  focusedPath,
  onFocusPath
}: TreemapProps) {
  const { t } = useI18n()
  const { ref: containerRef, width, height } = useContainerSize<HTMLDivElement>()

  const rects = useMemo(() => {
    if (width === 0 || height === 0 || items.length === 0) return []
    return computeTreemapRects(items, width, height)
  }, [items, width, height])

  // 用 ref 持有 rects，避免事件处理函数依赖频繁变化
  const rectsRef = useMemo(() => rects, [rects])

  /** 点击委托：同步左侧列表焦点 */
  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      const el = (e.target as HTMLElement).closest<HTMLElement>('[data-path]')
      const path = el?.dataset.path
      if (path) onFocusPath(path)
    },
    [onFocusPath]
  )

  /** 双击委托：钻入目录 / 打开文件 */
  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      const el = (e.target as HTMLElement).closest<HTMLElement>('[data-path]')
      const path = el?.dataset.path
      if (!path) return
      const item = rectsRef.find((i) => i.path === path)
      if (item) onDrillIn(item)
    },
    [rectsRef, onDrillIn]
  )

  return (
    <div
      ref={containerRef}
      data-treemap-container
      className="relative w-full h-full bg-transparent"
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
    >
      {scanning ? (
        <div className="flex items-center justify-center h-full w-full">
          <ScanOverlay progress={scanProgress} cancelling={cancelling} onCancel={onCancelScan} />
        </div>
      ) : rects.length === 0 ? (
        <div className="flex items-center justify-center h-full w-full">
          <span className="text-xs text-white/40">{t('analyze.treemap.empty')}</span>
        </div>
      ) : (
        rects.map((item) => (
          <RectBlock
            key={item.path}
            item={item}
            isSelected={item.path === focusedPath}
            isChecked={checkedPaths.has(item.path)}
          />
        ))
      )}
    </div>
  )
}
