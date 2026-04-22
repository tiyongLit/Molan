import { hierarchy, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy'
import type { TreemapItem } from '../../typings'

interface D3NodeData {
  name: string
  size?: number
  item?: TreemapItem
  children?: D3NodeData[]
}

/**
 * 最小视觉占比：相对最大项不足该比例的小项提升到该比例，
 * 保证可见但不夸大（对标 lemon-cleaner changeNumToLog 的 ratio>0.02 下限检查，但不丢弃条目）
 */
const MIN_VISUAL_RATIO = 0.015

/**
 * 极差压缩强度：值越小压缩越弱（1 = 不压缩）。
 * 旧值 5 在「小数据集 + 极大极差」场景（如根目录 13 项、极差百万倍）
 * 会把 0.3% 实际占比的项放大到 33%+ 视觉占比；降到 1.5 保持温和过渡，
 * 小项可见性交由 MIN_VISUAL_RATIO 下限保证
 */
const COMPRESS_FACTOR = 1.5

/**
 * 压缩大小极差，避免超大目录挤占小目录的显示空间。
 * 以「相对最大项的占比」为压缩域（避免极小项如 symlink 拉低归一化下限），
 * 配合 MIN_VISUAL_RATIO 下限。对标 lemon-cleaner 的 YMTreeMap log 变换思路
 */
function compressSize(size: number, maxSize: number): number {
  if (maxSize <= 0) return size
  const ratio = Math.min(Math.max(size / maxSize, 0), 1)
  const compressed = Math.pow(ratio, 1 / COMPRESS_FACTOR)
  const effective = Math.max(compressed, MIN_VISUAL_RATIO)
  return effective * maxSize
}

/**
 * 基于 d3-hierarchy Squarified Treemap 算法计算矩形布局
 */
export function computeTreemapRects(
  items: TreemapItem[],
  containerWidth: number,
  containerHeight: number
): TreemapItem[] {
  if (containerWidth <= 0 || containerHeight <= 0 || items.length === 0) {
    return []
  }

  const validItems = items.filter((item) => item.size > 0)
  if (validItems.length === 0) return []

  const maxSize = Math.max(...validItems.map((item) => item.size))
  // 多项即启用压缩：新算法以 maxSize 为参照，极小项（symlink 等）不会拉低归一化下限
  const useCompression = validItems.length > 1

  const processedItems = useCompression
    ? validItems.map((item) => ({
        ...item,
        displaySize: compressSize(item.size, maxSize)
      }))
    : validItems.map((item) => ({ ...item, displaySize: item.size }))

  const data: D3NodeData = {
    name: 'root',
    children: processedItems.map((item) => ({
      name: item.name,
      size: item.displaySize,
      item
    }))
  }

  const root = hierarchy(data)
    .sum((d) => d.size ?? 0)
    .sort((a, b) => (b.value || 0) - (a.value || 0))

  const layout = treemap<D3NodeData>()
    .tile(treemapSquarify.ratio(0.5))
    .size([containerWidth, containerHeight])
    .paddingInner(2)
    .paddingOuter(3)
    .round(true)

  layout(root)

  return (root.children || [])
    .map((node) => {
      const rectNode = node as HierarchyRectangularNode<D3NodeData>
      const item = rectNode.data.item
      if (!item) return null
      return {
        ...item,
        rect: {
          x: rectNode.x0,
          y: rectNode.y0,
          width: rectNode.x1 - rectNode.x0,
          height: rectNode.y1 - rectNode.y0
        }
      }
    })
    .filter(Boolean) as TreemapItem[]
}
