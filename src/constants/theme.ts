import type { CSSProperties } from 'react'

// ============================================================
// 全应用共享语义色板与页面主题变量
// 来源：Clean 页面视觉规范（勾选蓝 #3B82F6、警示橙 #FFBE46 等）
// ============================================================

/** 语义色板：全应用统一的硬编码颜色出口，禁止在业务组件中直接写色值 */
export const SEMANTIC_COLORS = {
  /** 勾选蓝（全应用统一规范：选中态边框与背景） */
  accentBlue: '#3B82F6',
  /** 扫描中/已选中高亮橙 */
  warningYellow: '#FFBE46',
  /** "很干净"/成功态绿 */
  successGreen: '#33D39D',
  /** 错误文案红 */
  dangerRed: '#FCA5A5',
  /** 谨慎清理橙红 */
  cautiousOrange: '#E6704C',
} as const

/** 页面半透明主题变量（Clean/Optimize 等浅色玻璃页共用，避免每次 render 重建） */
export const PAGE_THEME_VARS: CSSProperties = {
  '--bg-page': 'rgba(255, 255, 255, 0.03)',
  '--bg-card': 'rgba(255, 255, 255, 0.06)',
  '--border': 'rgba(255, 255, 255, 0.1)',
} as CSSProperties
