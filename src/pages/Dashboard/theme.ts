// Dashboard 托盘气泡专属主题（派生自 src/layout/themeColors.ts 的渐变方案）。
//
// Dashboard 是独立子窗口（非路由页面），使用 cmmPalette.dashboard
// 青蓝调色板：#4e90aa → #285a73，160deg 线性渐变，以 deep 为基调的柔和过渡。
//
// 只影响 Dashboard 目录内组件（内联 style / Tailwind arbitrary value），
// 不修改全局 CSS 变量，Uninstall/Analyze/Optimize 等模块零影响。

import { cmmPalette } from '@/layout/themeColors'

const palette = cmmPalette.dashboard
const [br, bg, bb] = palette.bloom
const [dr, dg, db] = palette.deep

export const dashTheme = {
  /** 页面背景：160deg 亮→深两段渐变（bg4.html 复刻方案） */
  pageBg: `linear-gradient(160deg, rgb(${br},${bg},${bb}) 0%, rgb(${dr},${dg},${db}) 100%)`,
  /** 卡片底：白 15% 半透明（渐变底上的层级对比，微边框增加分离感） */
  card: 'rgba(255,255,255,0.15)',
  cardBorder: 'rgba(255,255,255,0.08)',
  /** 文字三级：白 / 85% / 65%（毛玻璃底上保证可读性） */
  textPrimary: '#FFFFFF',
  textSecondary: 'rgba(255,255,255,0.85)',
  textTertiary: 'rgba(255,255,255,0.65)',
  /** accent（bloom 天蓝）与按钮渐变（hover 加深） */
  accent: palette.accent,
  accentSoft: '#8ec8dd',
  btnGrad: `linear-gradient(135deg, ${palette.accent} 0%, rgba(${dr},${dg},${db},0.85) 100%)`,
  btnGradHover: `linear-gradient(135deg, rgba(${br},${bg},${bb},0.85) 0%, rgba(${dr},${dg},${db},0.95) 100%)`,
  /** 进度轨道：accent 同色系低饱和（从 palette 动态派生） */
  track: `rgba(${br},${bg},${bb},0.2)`,
  /** 内存水位色块：accent 35% 透明（柠檬 LMMemoryCellView 水位语法） */
  waterFill: `rgba(${br},${bg},${bb},0.45)`,
  /** 正向反馈绿 */
  success: '#4ade80',
  successBg: 'rgba(74,222,128,0.14)',
  /** 警告黄（中等负载/资源紧张） */
  warn: '#fbbf24',
  warnBg: 'rgba(251,191,36,0.14)',
  /** 严重红（高负载/资源严重不足） */
  critical: '#f87171',
  criticalBg: 'rgba(248,113,113,0.14)'
}
