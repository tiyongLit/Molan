import { FILE_TYPE_DESCS, BUNDLE_TYPE_DESCS } from '@/constants/shared'
import type { MoleAnalyzeEntry } from '@/types/mole'

/**
 * EntryRow 副标题查找 — 统一槽位，按行类型与模式决定文案。
 *
 * 对标 lemon-cleaner LMSpaceTableRowView 的差异化文案设计
 * （目录 "X 项" / 文件走类型描述），类型文案来自 build.rs 生成的
 * 单一事实来源表（constants.rs），保证前后端一致。
 *
 * 回退层级（优先级从上到下）：
 *   1. Top 20 模式          → 完整路径（现状保留）
 *   2. symlink              → "符号链接"（后端 is_symlink 显式标记）
 *   3. 目录 + bundle 后缀    → BUNDLE_TYPE_DESCS（"应用程序"/"框架"…，替代统计文案）
 *   4. 普通目录             → "X 项"（child_files + child_dirs + child_links，
 *                             柠檬式单数字；total=0 或旧缓存无统计 → 空白）
 *   5. 文件 + 扩展名命中     → FILE_TYPE_DESCS（"ZIP 归档"/"PNG 图像"…）
 *   6. 其余（未知类型）      → 空字符串（对齐柠檬"未知类型不显示"）
 */
export function entrySubtitle(entry: MoleAnalyzeEntry, showPath: boolean): string {
  if (showPath) return entry.path
  if (entry.is_symlink) return '符号链接'

  if (entry.is_dir) {
    // bundle 叶子捷径：优先展示 bundle id（叶子无子项统计，避免误导的 "0 项"）
    if (entry.is_bundle_leaf && entry.bundle_id) return entry.bundle_id
    // 目录型 bundle（.app/.framework…）按名称后缀查表，命中即替代统计文案
    const name = entry.name.toLowerCase()
    for (const [suffix, desc] of Object.entries(BUNDLE_TYPE_DESCS)) {
      if (name.endsWith(suffix)) return desc
    }
    // 普通目录：柠檬式 "X 项"（直接子项总数；0 或旧缓存缺失时不显示）
    const total =
      (entry.child_files ?? 0) + (entry.child_dirs ?? 0) + (entry.child_links ?? 0)
    return total > 0 ? `${total} 项` : ''
  }

  // 普通文件：取最后一段扩展名查表（".tar.gz" → "gz" → "GZip 归档"，与系统 UTI 行为一致）
  const dot = entry.name.lastIndexOf('.')
  if (dot > 0 && dot < entry.name.length - 1) {
    const ext = entry.name.slice(dot + 1).toLowerCase()
    return FILE_TYPE_DESCS[ext] ?? ''
  }
  return ''
}
