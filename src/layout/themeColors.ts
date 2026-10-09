// ── 页面渐变主题色 ──
//
// 渐变结构（复刻 bg4.html 方案）：
//   单一线性渐变 160deg，从亮色（0%）到深色（100%）
//   不再使用径向发光与中间色，过渡干净纯粹
//
// 🎨 微调记录：保留暗色基调（护眼偏好），亮度只 +3% 左右；
//   "去旧"靠提纯：深色端饱和度 +10%~15%，同深度下颜色更干净、更有光泽，
//   而不是靠加亮。色相身份全部保留。结构不变：每路由独立配色、
//   切路由渐变过渡、160deg 两段式。
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
// 路由 → bg4 变量 → 颜色（微调后；原值见各行注释）
//
// home       → shredder   （碎纸机）      #68bfd4 → #334491
// clean      → mail       （邮件附件）    #7eafe7 → #304088
// uninstall  → lens       （空间透镜）    #47c2ab → #355297
// optimize   → extensions  （扩展）       #9e8cde → #4b2a84
// analyze    → large-files（大型和旧文件）#ce7f78 → #663a8d
// dashboard  → dashboard  （托盘仪表盘）    #4f9cbb → #286a8a
// ─────────────────────────────────────────────────────

// ── shredder：碎纸机（钢蓝色）→ home ──
const SHREDDER_BLOOM: RGB = [104, 191, 212]  // #68bfd4（原 #60b4c8，亮 +4%）
const SHREDDER_DEEP: RGB = [51, 68, 145]     // #334491（原 #384478，同深度提纯）

// ── mail：邮件附件（蓝色）→ clean ──
const MAIL_BLOOM: RGB = [126, 175, 231] // #7eafe7（原 #73a6e0，亮 +3.5%）
const MAIL_DEEP: RGB = [48, 64, 136]    // #304088（原 #354070，同深度提纯）

// ── lens：空间透镜（青绿色）→ uninstall ──
const LENS_BLOOM: RGB = [71, 194, 171]   // #47c2ab（原 #47b09c，亮 +3.6%）
const LENS_DEEP: RGB = [53, 82, 151]     // #355297（原 #3a5082，同深度提纯）

// ── extensions：扩展（紫色）→ optimize ──
const EXTENSIONS_BLOOM: RGB = [158, 140, 222] // #9e8cde（原 #9683d8，亮 +3%）
const EXTENSIONS_DEEP: RGB = [75, 42, 132]    // #4b2a84（原 #3c2761，原为全场最暗，略多提）

// ── large-files：大型和旧文件（橙红色）→ analyze ──
const LARGEFILES_BLOOM: RGB = [206, 127, 120] // #ce7f78（原 #c47972，亮 +3%）
const LARGEFILES_DEEP: RGB = [102, 58, 141]   // #663a8d（原 #5c4078，灰紫去灰）

// ── dashboard：托盘仪表盘（青蓝，渐变以 deep 为基调）──
const DASHBOARD_BLOOM: RGB = [79, 156, 187] // #4f9cbb（原 #4e90aa，亮 +3.6%）
const DASHBOARD_DEEP: RGB  = [40, 106, 138] // #286a8a（原 #285a73，同深度提纯）

export const cmmPalette: Record<string, ThemeColors> = {
  home: { bloom: SHREDDER_BLOOM, deep: SHREDDER_DEEP, accent: '#68bfd4' },
  clean: { bloom: MAIL_BLOOM, deep: MAIL_DEEP, accent: '#7eafe7' },
  uninstall: { bloom: LENS_BLOOM, deep: LENS_DEEP, accent: '#47c2ab' },
  optimize: { bloom: EXTENSIONS_BLOOM, deep: EXTENSIONS_DEEP, accent: '#9e8cde' },
  analyze: { bloom: LARGEFILES_BLOOM, deep: LARGEFILES_DEEP, accent: '#ce7f78' },
  dashboard: { bloom: DASHBOARD_BLOOM, deep: DASHBOARD_DEEP, accent: '#4f9cbb' },
}

// ─────────────────────────────────────────────────────
// 🔧 切换方案：改下面一行即可
// ─────────────────────────────────────────────────────

export const activePalette = cmmPalette
export const navAccents = Object.fromEntries(
  Object.entries(activePalette).map(([k, v]) => [k, v.accent])
)
