// ── 页面渐变主题色 ──
//
// 渐变结构（CleanMyMac X 截图吸色定标，2026-10-10）：
//   单一线性渐变 160deg，从主题色（0%）到统一深蓝紫（100%）
//   结构不变：每路由独立配色、切路由渐变过渡、160deg 两段式。
//
// 🎨 CMM X 定标规律（13 张截图逐像素吸色验证）：
//   顶色 = 中低饱和（s 21~47%）、中明度（l 46~63%）的功能主题色，不亮不奶；
//   底色 = 全站统一的宇宙深蓝紫 #3c3f72（h237 s31 l34），所有页面收敛到它；
//   两段 RGB 线性插值的 hue 迁移路径（主题色 → 237°）与 CMM X 实测中段
//   逐点吻合（误差 < 3°），证明 CMM X 就是两段渐变，端点取值讲究而已。
//
// 🔧 调整颜色只改下面的常量即可，不用动组件代码
// ─────────────────────────────────────────────────────

export type RGB = [number, number, number]

export interface ThemeColors {
  bloom: RGB      // 渐变起始色（主题色，0%）
  mid?: RGB      // 可选中段锁定色（45% stop）：仅当页面重要信息落在渐变
                 // 中段（如首页居中列）时使用，避免 bloom→deep 数学插值
                 // 中点的低饱和灰；缺省时自动取两端 45% 插值（等价两段）
  deep: RGB       // 渐变结束色（统一深蓝紫，100%）
  accent: string  // 侧边栏图标高亮色
}

// ─────────────────────────────────────────────────────
// CMM X 吸色定标（160deg 两段渐变）
// 路由 → 对应 CMM X 页面 → 顶色实测值

// home       → 恶意软件（橙红）     #c67973，中段锁定 #bf6980
// clean      → 邮件附件（蓝）       #708dbf
// uninstall  → 空间透镜（青绿）     #45a78a
// optimize   → 扩展（紫）           #9877b7
// analyze    → 废纸篓浅青提亮       #5bb4af
// dashboard  → 无对应页，取卸载器蓝 #6493b9
//
// 选取约束：青绿族三席（h162/170/177）挤在同族，按菜单切页会像没换色，
//   只留 uninstall（空间透镜语义绑定最强）与 analyze（用户指定保留）。
//   home 用恶意软件橙红 #c67973（大型旧文件 #c6756f 为同族备选，hue 差 0°），
//   不与相邻 clean 蓝 h218 撞色。analyze 在实测 #50aca8 基础上仅明度
//   l49→l53（hue/饱和不变）。紫族三个实测值全撞 optimize，未启用。
// ─────────────────────────────────────────────────────

// ── 恶意软件（橙红色）→ home ──
const SHREDDER_BLOOM: RGB = [198, 121, 115]  // #c67973（实测恶意软件页顶色 h4 s42 l61）
// 中段锁定色：首页「标题+磁盘卡+按钮」列垂直居中，正压两段插值中点；
// 橙红→深蓝紫路径（h4→343→308…→237）的数学中点是 h338 s20% 的灰玫红，
// 同样会压暗信息区（布局与 CMM X 不同：它的重要信息在顶部/底部）。
// 本值位于该路径上，饱和度/明度高于插值中点，保住橙红气质。
const SHREDDER_MID: RGB = [191, 105, 128]  // #bf6980（h338 s40 l58）
const SHREDDER_DEEP: RGB = [60, 63, 114]     // #3c3f72（全站统一深蓝紫）

// ── 邮件附件（蓝色）→ clean ──
const MAIL_BLOOM: RGB = [112, 141, 191] // #708dbf（实测 h218 s38 l59）
const MAIL_DEEP: RGB = [60, 63, 114]    // #3c3f72

// ── 空间透镜（青绿色）→ uninstall ──
const LENS_BLOOM: RGB = [69, 167, 138]   // #45a78a（实测 h162 s42 l46）
const LENS_DEEP: RGB = [60, 63, 114]     // #3c3f72

// ── 扩展（紫色）→ optimize ──
const EXTENSIONS_BLOOM: RGB = [152, 119, 183] // #9877b7（实测 h271 s31 l59）
const EXTENSIONS_DEEP: RGB = [60, 63, 114]    // #3c3f72

// ── 废纸篓浅青（提亮版）→ analyze ──
const LARGEFILES_BLOOM: RGB = [91, 180, 175] // #5bb4af（实测 #50aca8 明度 l49→l53，hue/饱和不变）
const LARGEFILES_DEEP: RGB = [60, 63, 114]    // #3c3f72

// ── 托盘仪表盘（蓝色，无 CMM X 对应页，借卸载器蓝）──
const DASHBOARD_BLOOM: RGB = [100, 147, 185] // #6493b9（实测卸载器页顶色 h207 s38 l56）
const DASHBOARD_DEEP: RGB  = [60, 63, 114]   // #3c3f72

export const cmmPalette: Record<string, ThemeColors> = {
  home: { bloom: SHREDDER_BLOOM, mid: SHREDDER_MID, deep: SHREDDER_DEEP, accent: '#c67973' },
  clean: { bloom: MAIL_BLOOM, deep: MAIL_DEEP, accent: '#708dbf' },
  uninstall: { bloom: LENS_BLOOM, deep: LENS_DEEP, accent: '#45a78a' },
  optimize: { bloom: EXTENSIONS_BLOOM, deep: EXTENSIONS_DEEP, accent: '#9877b7' },
  analyze: { bloom: LARGEFILES_BLOOM, deep: LARGEFILES_DEEP, accent: '#5bb4af' },
  dashboard: { bloom: DASHBOARD_BLOOM, deep: DASHBOARD_DEEP, accent: '#6493b9' },
}

// ─────────────────────────────────────────────────────
// 🔧 切换方案：改下面一行即可
// ─────────────────────────────────────────────────────

export const activePalette = cmmPalette
export const navAccents = Object.fromEntries(
  Object.entries(activePalette).map(([k, v]) => [k, v.accent])
)
