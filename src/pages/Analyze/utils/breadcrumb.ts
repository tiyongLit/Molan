import type { BreadcrumbItem } from '../typings'

/** 被折叠段（携带原始索引，供下拉菜单跳转） */
export interface CollapsedItem extends BreadcrumbItem {
  originalIdx: number
}

export interface VisItem {
  name: string
  path: string
  /** 在原始 items 中的索引；ellipsis 项为 -1 */
  originalIdx: number
  ellipsis?: boolean
  /** ellipsis 项：被折叠的段（含原始索引），供下拉菜单展示/跳转 */
  collapsed?: CollapsedItem[]
}

const FIRST_MIN_WIDTH = 70
const LAST_MIN_WIDTH = 90
const ICON_WIDTH = 18
const SEG_MAX_WIDTH = 180
/** 分隔符占宽：RightOutlined 8px + margin-inline 4px×2 */
const SEP_WIDTH = 20
/** 省略号项占宽（图标 + 边距） */
const ELLIPSIS_WIDTH = 36

/**
 * 估算单段宽度 — 区分字符宽度：
 * ASCII ≈ 7px/字符，非 ASCII（中文/日文等全宽字符）≈ 12px/字符（13px 字号实测）。
 * 旧版统一 name.length * 8 对中文低估约 1/3，导致"算法认为放得下、实际溢出"。
 */
function estSegWidth(name: string): number {
  let units = 0
  for (const ch of name) units += ch.charCodeAt(0) > 0x7f ? 12 : 7
  return ICON_WIDTH + Math.max(units, 36) + 14
}

/** 生成带 originalIdx 的可见项 */
function withIdx(items: BreadcrumbItem[]): VisItem[] {
  return items.map((it, i) => ({ ...it, originalIdx: i }))
}

/**
 * 根据容器宽度计算面包屑可见项 — 对齐 macOS NSPathControl（Finder）折叠策略：
 * - 未超宽：全部显示
 * - 超宽：首段 + 省略号（下拉可展开）+ 靠近末尾的中间段 + 末段
 *   （保留尾部中间段：越深的路径上下文越相关，与 Finder 认知一致）
 * - 省略号项携带被折叠段（collapsed），供下拉菜单跳转任意层级
 */
export function computeVisibleItems(items: BreadcrumbItem[], containerWidth: number): VisItem[] {
  const n = items.length
  if (containerWidth <= 0 || n === 0) return withIdx(items)
  if (n <= 2) return withIdx(items)

  const widths = items.map((item, i) => {
    const est = Math.min(estSegWidth(item.name), SEG_MAX_WIDTH)
    if (i === 0) return Math.max(est, FIRST_MIN_WIDTH)
    if (i === n - 1) return Math.max(est, LAST_MIN_WIDTH)
    return est
  })

  const totalWidth = widths.reduce((a, b) => a + b, 0) + SEP_WIDTH * (n - 1)
  if (totalWidth <= containerWidth) return withIdx(items)

  const result: VisItem[] = [{ ...items[0], originalIdx: 0 }]

  // 中间段可用宽度 = 容器 - 首段 - 末段 - 首尾分隔符 - 省略号（按已折叠保守预留）
  const middleAvail = containerWidth - widths[0] - widths[n - 1] - SEP_WIDTH * 3 - ELLIPSIS_WIDTH

  if (middleAvail < 50) {
    // 极窄：中间段全折叠
    result.push({
      name: '...',
      path: '',
      originalIdx: -1,
      ellipsis: true,
      collapsed: items.slice(1, n - 1).map((it, i) => ({ ...it, originalIdx: i + 1 }))
    })
  } else {
    // 从尾部往前保留能放下的中间段，其余折叠为省略号
    let accumulated = 0
    let keepFrom = n - 1
    for (let i = n - 2; i >= 1; i--) {
      if (accumulated + widths[i] > middleAvail) break
      accumulated += widths[i]
      keepFrom = i
    }
    if (keepFrom > 1) {
      result.push({
        name: '...',
        path: '',
        originalIdx: -1,
        ellipsis: true,
        collapsed: items
          .slice(1, keepFrom)
          .map((it, i) => ({ ...it, originalIdx: i + 1 }))
      })
    }
    for (let i = keepFrom; i < n - 1; i++) result.push({ ...items[i], originalIdx: i })
  }

  result.push({ ...items[n - 1], originalIdx: n - 1 })
  return result
}
