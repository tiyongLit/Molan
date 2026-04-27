import type { MoleCleanItem } from '@/types/mole'

// ============================================================
// 扫描阶段 → 前端分组映射（纯函数，无 React 依赖）
// 数据来源：src-tauri/src/controllers/clean.rs 中 emit_cleanup_phase_result 调用
// ============================================================

// 后端实际 section 列表（按 src-tauri/src/controllers/clean.rs 中 start_section 顺序）。
// section 才是后端逻辑上的"扫描小节"——同一 section 内可能含多个 phase。
// 用 section 维度做"进行中"判定比 phase 维度更准确。
const SECTION_ORDER: string[] = [
  'System',                  // 1. system.local_snapshots, system
  'User essentials',         // 2. user.essentials, user.finder_metadata
  'App caches',              // 3. app.caches
  'Browsers',                // 4. browser.caches
  'Cloud & Office',          // 5. cloud.storage, office.caches
  'Developer tools',         // 6. dev.tools
  'Applications',            // 7. applications
  'Virtualization',          // 8. virtualization
  'Application Support',     // 9. app_support.logs
  'App leftovers',           // 10. orphaned.data, orphaned.system_services, orphaned.container_stubs
  'Apple Silicon',           // 11. apple_silicon.caches
  'Device',                  // 12. device.firmware
  'Time Machine',            // 13. system.time_machine
  'Large files',             // 14. large_files
  'System Data',             // 15. system_data.hints
  'Project artifacts',       // 16. project_artifacts
]

// section → 所属 group（一次映射，section 维度直接判定）
const SECTION_TO_GROUP: Record<string, string> = {
  'System': 'system',
  'User essentials': 'app',
  'App caches': 'app',
  'Browsers': 'browser_cloud',
  'Cloud & Office': 'browser_cloud',
  'Developer tools': 'dev',
  'Applications': 'app',
  'Virtualization': 'dev',
  'Application Support': 'app',
  'App leftovers': 'app',
  'Apple Silicon': 'system',
  'Device': 'device',
  'Time Machine': 'system',
  'Large files': 'large',
  'System Data': 'system',
  'Project artifacts': 'app',
}

export type ScanGroupStatus = 'pending' | 'scanning' | 'done'

/**
 * 根据后端 section 完成情况计算前端 group 的扫描状态。
 * 后端是"完成型"事件：phase result 在 section 结束后才推。
 * 启发式：SECTION_ORDER 中第一个未完成的 section 所属的 group = scanning。
 *  - 该 group 之前的 group：已结束 → done
 *  - 该 group：scanning
 *  - 该 group 之后的 group：pending
 */
export function computeGroupStatusBySection(
  completedSections: Set<string>,
  groupId: string
): ScanGroupStatus {
  const firstUndoneSection = SECTION_ORDER.find(s => !completedSections.has(s))
  // 所有 section 都完成 → 所有 group 都是 done
  if (!firstUndoneSection) return 'done'
  // group 是否已经全部完成（所有属于该 group 的 section 都在 completedSections）
  const groupSections = SECTION_ORDER.filter(s => SECTION_TO_GROUP[s] === groupId)
  if (groupSections.every(s => completedSections.has(s))) return 'done'
  // group 是否包含"第一个未完成 section"
  const firstUndoneGroup = SECTION_TO_GROUP[firstUndoneSection]
  return firstUndoneGroup === groupId ? 'scanning' : 'pending'
}

// 估算扫描总阶段数（用于进度百分比近似，非精确）
export const ESTIMATED_SCAN_PHASES = 17

// ============================================================
// 工具函数
// ============================================================

/** 组合键：categoryId::itemId（勾选集合的 key） */
export function selKey(catId: string, itemId: string): string {
  return `${catId}::${itemId}`
}

/** 判断子项是否计入「可清理总量」：仅统计真实可清理项（status=cleanable），
 * 排除 info 提示项（如 system_data_clues / large_files）与估算项（project_artifacts）。 */
export const isCountableCleanItem = (item: MoleCleanItem): boolean =>
  item.status === 'cleanable' && item.categoryId !== 'project_artifacts'
