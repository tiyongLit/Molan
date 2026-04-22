import { createContext, useContext } from 'react'
import type { MoleAnalyzeEntry, MoleAnalyzeResult } from '@/types/mole'
import type { TreemapItem } from '../typings'
import type { ScanProgress } from '../hooks/useAnalyzeData'

/**
 * 数据 Context — 扫描/导航/图标加载时更新（中频）。
 *
 * 包含所有「只读派生数据」：entries / treemapItems / iconMap / 加载状态。
 * 与 ActionContext 分离的关键收益：图标异步解析、扫描进度推送只触发数据消费者
 * （EntryList / Treemap / ScanOverlay）重渲染，不影响 Toolbar 等操作类组件。
 */
export interface AnalyzeDataContextValue {
  /** 当前目录的条目列表（已经过后端排序） */
  entries: MoleAnalyzeEntry[]
  /** 当前目录总文件数（含子目录） */
  totalFiles: number
  /** 路径 → 图标（原生 SVG data URI 优先，emoji 兜底） */
  iconMap: Record<string, string>
  /** Treemap 可视化条目（已截断到 TREEMAP_MAX_ITEMS） */
  treemapItems: TreemapItem[]
  /** 主扫描进行中（首次 scanRoot / 重新扫描 / bundle 叶子 fallback） */
  browseLoading: boolean
  /** bundle 叶子钻取的按需扫描中（前端据此渲染骨架屏） */
  bundleLoading: boolean
  /** 取消请求已发出、等后端 walker 退出 */
  cancelling: boolean
  /** 扫描进度（仅 browseLoading 期间非 null） */
  scanProgress: ScanProgress | null
  /** overview 阶段的根目录结果（immutable，来自父级 props） */
  overviewResult: MoleAnalyzeResult
}

export const AnalyzeDataContext = createContext<AnalyzeDataContextValue | null>(null)

export function useAnalyzeData(): AnalyzeDataContextValue {
  const ctx = useContext(AnalyzeDataContext)
  if (!ctx) throw new Error('useAnalyzeData must be used within AnalyzeProvider')
  return ctx
}
