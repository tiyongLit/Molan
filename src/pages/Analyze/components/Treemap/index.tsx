import { useMemo, useState, useCallback } from 'react'
import { RectBlock } from './RectBlock'
import { computeTreemapRects } from './computeTreemap'
import { useContainerSize } from '@/hooks/useContainerSize'
import { ScanOverlay } from './ScanOverlay'
import type { TreemapItem, ContextMenuItem } from '../../typings'
import type { ScanProgress } from '../../hooks/useAnalyzeData'

interface TreemapProps {
  items: TreemapItem[]
  onDrillIn: (item: TreemapItem) => void
  scanning: boolean
  scanProgress: ScanProgress | null
  /** 勾选集合（path 对齐）— treemapItems 经上限截断后索引与 activeData 不再对齐，统一按 path 匹配 */
  checkedPaths: Set<string>
  /** 当前焦点条目路径（来自左侧列表 focusedIdx）— 键盘 ↑↓ 时同步高亮对应方格 */
  focusedPath: string | null
  /** 单击方格回调 — 同步左侧列表焦点 */
  onFocusPath: (path: string) => void
  buildContextMenu?: (item: TreemapItem) => { items: ContextMenuItem[] }
}

export function Treemap({
  items,
  onDrillIn,
  scanning,
  scanProgress,
  checkedPaths,
  focusedPath,
  onFocusPath,
  buildContextMenu
}: TreemapProps) {
  const { ref: containerRef, width, height } = useContainerSize<HTMLDivElement>()
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null)

  const rects = useMemo(() => {
    if (width === 0 || height === 0 || items.length === 0) return []
    return computeTreemapRects(items, width, height)
  }, [items, width, height])

  const handleMouseEnter = useCallback((idx: number) => setHoveredIdx(idx), [])
  const handleMouseLeave = useCallback(() => setHoveredIdx(null), [])

  return (
    <div ref={containerRef} className="relative w-full h-full bg-transparent">
      {scanning ? (
        <div className="flex items-center justify-center h-full w-full">
          <ScanOverlay progress={scanProgress} />
        </div>
      ) : rects.length === 0 ? (
        <div className="flex items-center justify-center h-full w-full">
          <span className="text-xs text-white/40">无数据</span>
        </div>
      ) : (
        rects.map((item, idx) => (
          <RectBlock
            key={item.path || item.name}
            item={item}
            isSelected={item.path === focusedPath}
            isHovered={idx === hoveredIdx}
            isChecked={checkedPaths.has(item.path)}
            onClick={() => onFocusPath(item.path)}
            onDoubleClick={() => onDrillIn(item)}
            onMouseEnter={() => handleMouseEnter(idx)}
            onMouseLeave={handleMouseLeave}
            buildContextMenu={buildContextMenu}
          />
        ))
      )}
    </div>
  )
}
