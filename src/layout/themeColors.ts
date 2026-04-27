// ── 页面渐变主题色 ──
//
// 渐变结构（复刻 bg4.html 方案）：
//   单一线性渐变 160deg，从亮色（0%）到深色（100%）
//   不再使用径向发光与中间色，过渡干净纯粹
//
// 🔧 调整颜色只改下面的常量即可，不用动组件代码
// ─────────────────────────────────────────────────────

export type RGB = [number, number, number]

export interface ThemeColors {
  bloom: RGB      // 渐变起始色（亮色，0%）
  deep: RGB       // 渐变结束色（深色，100%）
  accent: string  // 侧边栏图标高亮色
}

// ─────────────────────────────────────────────────────
// bg4.html 复刻方案（160deg 两段渐变）
// 路由 → bg4 变量 → 颜色（顺序以 README L67-83 为准）
//
// home       → shredder   （碎纸机）      #60b4c8 → #384478
// clean      → mail       （邮件附件）    #73a6e0 → #354070
// uninstall  → lens       （空间透镜）    #47b09c → #3a5082
// optimize   → extensions  （扩展）       #9683d8 → #3c2761
// analyze    → large-files（大型和旧文件）#c47972 → #5c4078
// dashboard  → dashboard  （托盘仪表盘）    #4e90aa → #285a73
// ─────────────────────────────────────────────────────

// ── shredder：碎纸机（钢蓝色）→ home ──
const SHREDDER_BLOOM: RGB = [96, 180, 200]   // #60b4c8
const SHREDDER_DEEP: RGB = [56, 68, 120]    // #384478

// ── mail：邮件附件（蓝色）→ clean ──
const MAIL_BLOOM: RGB = [115, 166, 224] // #73a6e0
const MAIL_DEEP: RGB = [53, 64, 112]    // #354070

// ── lens：空间透镜（青绿色）→ uninstall ──
const LENS_BLOOM: RGB = [71, 176, 156]   // #47b09c
const LENS_DEEP: RGB = [58, 80, 130]    // #3a5082

// ── extensions：扩展（紫色）→ optimize ──
const EXTENSIONS_BLOOM: RGB = [150, 131, 216] // #9683d8
const EXTENSIONS_DEEP: RGB = [60, 39, 97]    // #3c2761

// ── large-files：大型和旧文件（橙红色）→ analyze ──
const LARGEFILES_BLOOM: RGB = [196, 121, 114] // #c47972
const LARGEFILES_DEEP: RGB = [92, 64, 120]    // #5c4078

// ── dashboard：托盘仪表盘（青蓝，渐变以 deep 为基调）──
const DASHBOARD_BLOOM: RGB = [78, 144, 170] // #4e90aa
const DASHBOARD_DEEP: RGB  = [40, 90, 115]  // #285a73

export const cmmPalette: Record<string, ThemeColors> = {
  home: { bloom: SHREDDER_BLOOM, deep: SHREDDER_DEEP, accent: '#60b4c8' },
  clean: { bloom: MAIL_BLOOM, deep: MAIL_DEEP, accent: '#73a6e0' },
  uninstall: { bloom: LENS_BLOOM, deep: LENS_DEEP, accent: '#47b09c' },
  optimize: { bloom: EXTENSIONS_BLOOM, deep: EXTENSIONS_DEEP, accent: '#9683d8' },
  analyze: { bloom: LARGEFILES_BLOOM, deep: LARGEFILES_DEEP, accent: '#c47972' },
  dashboard: { bloom: DASHBOARD_BLOOM, deep: DASHBOARD_DEEP, accent: '#4e90aa' },
}

// ─────────────────────────────────────────────────────
// 🔧 切换方案：改下面一行即可
// ─────────────────────────────────────────────────────

export const activePalette = cmmPalette
export const navAccents = Object.fromEntries(
  Object.entries(activePalette).map(([k, v]) => [k, v.accent])
)
