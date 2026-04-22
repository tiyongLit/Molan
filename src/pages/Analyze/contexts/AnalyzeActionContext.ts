import { createContext, useContext } from 'react'
import type { MoleAnalyzeEntry } from '@/types/mole'
import type { ContextMenuItem, MenuEntry } from '../typings'

/**
 * 操作 Context — 全部为稳定回调 + 极低频状态（trashing / filterQuery / filtering）。
 *
 * 与 DataContext 分离的关键收益：扫描进度推送、图标解析等数据层变化
 * 不会触发操作类消费者（Toolbar 的「重新扫描」按钮、BottomBar 的「移到废纸篓」按钮）重渲染。
 *
 * 注意：filterQuery 在用户输入时高频变化，会让所有 ActionContext 消费者重渲染。
 * 当前消费者集合（Toolbar / EntryList / BottomBar / BrowsingView）规模可控，
 * 若未来发现输入卡顿，可把 filter 状态独立成第 5 个 Context。
 */
export interface AnalyzeActionContextValue {
  // ── 扫描操作 ──
  /** 强制重扫当前路径（删除文件后刷新 / 用户手动刷新） */
  refreshPath: (path: string) => Promise<void>
  /** 取消当前扫描并返回 overview */
  cancelScanAndExit: () => Promise<void>

  // ── 条目操作 ──
  /** 双击条目：目录钻入 / 文件用系统默认应用打开 */
  onActivate: (entry: MoleAnalyzeEntry) => void
  /** 构建右键菜单项（受保护条目自动隐藏「移到废纸篓」） */
  buildContextMenu: (entry: MenuEntry) => { items: ContextMenuItem[] }

  // ── 删除操作 ──
  trashSelected: () => void
  trashEntry: (entry: MenuEntry) => void
  trashing: boolean

  // ── 过滤 ──
  filterQuery: string
  filtering: boolean
  setFilterQuery: (q: string) => void
  setFiltering: (v: boolean) => void
}

export const AnalyzeActionContext = createContext<AnalyzeActionContextValue | null>(null)

export function useAnalyzeAction(): AnalyzeActionContextValue {
  const ctx = useContext(AnalyzeActionContext)
  if (!ctx) throw new Error('useAnalyzeAction must be used within AnalyzeProvider')
  return ctx
}
