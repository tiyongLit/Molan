import { createContext, useContext } from 'react'
import type { BreadcrumbItem } from '../typings'

/**
 * 导航 Context — 仅在路径变化时更新（极低频）。
 *
 * 拆分动机：原 AnalyzeContext 把导航与数据/图标/操作/过滤混在一起，
 * 任何字段变化都会让 Toolbar/PathBreadcrumb 等纯导航消费者重渲染。
 * 独立成 Context 后，勾选/输入过滤词/图标加载都不会触发导航消费者重渲染。
 */
export interface AnalyzeNavContextValue {
  currentPath: string
  canGoBack: boolean
  canGoForward: boolean
  goBack: () => void
  goForward: () => void
  drillIn: (path: string) => void
  breadcrumbJump: (idx: number) => void
  breadcrumbItems: BreadcrumbItem[]
  /** 返回 overview 阶段（来自父级 props，引用稳定） */
  backToOverview: () => void
  /** 切换根目录（来自父级 props，引用稳定） */
  switchRoot: (path: string) => void
}

export const AnalyzeNavContext = createContext<AnalyzeNavContextValue | null>(null)

export function useAnalyzeNav(): AnalyzeNavContextValue {
  const ctx = useContext(AnalyzeNavContext)
  if (!ctx) throw new Error('useAnalyzeNav must be used within AnalyzeProvider')
  return ctx
}
