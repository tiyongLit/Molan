import type { MoleOptimizeDiagnostics, MoleOptimizeResult, MoleOptimizeSystemInfo, MoleOptimizeTask } from '@/types/mole'

// ============================================================
// Optimize 静态阶段 Mock 数据
// 任务目录对齐 Mole lib/optimize/catalog.sh（21 个安全任务，顺序一致）。
// 接后端后整体替换为 tauri.mole_optimize({ dry_run: true }) 的返回。
// ============================================================

export const OPTIMIZE_MOCK_TASKS: MoleOptimizeTask[] = [
  { id: 'system_maintenance', name: 'DNS 与 Spotlight 检查', description: '刷新 DNS 缓存并检查 Spotlight 状态', safe: true, status: 'pending' },
  { id: 'cache_refresh', name: 'Finder 缓存刷新', description: '刷新 QuickLook 缩略图与图标服务缓存', safe: true, status: 'pending' },
  { id: 'saved_state_cleanup', name: '应用状态清理', description: '清理 30 天前的旧应用状态', safe: true, status: 'pending' },
  { id: 'fix_broken_configs', name: '损坏配置修复', description: '修复损坏的偏好设置文件', safe: true, status: 'pending' },
  { id: 'network_optimization', name: '网络缓存刷新', description: '优化 DNS 缓存并重启 mDNSResponder', safe: true, status: 'pending' },
  { id: 'sqlite_vacuum', name: '数据库优化', description: '压缩 Mail、Safari 与 Messages 的 SQLite 数据库', safe: true, status: 'pending' },
  { id: 'launch_services_rebuild', name: 'LaunchServices 修复', description: '修复“打开方式”菜单与文件关联', safe: true, status: 'pending' },
  { id: 'prevent_network_dsstore', name: '防止网络卷 .DS_Store', description: '停止在网络与 USB 卷上写入 .DS_Store', safe: true, status: 'pending' },
  { id: 'legacy_overrides_audit', name: '旧版覆盖清理', description: '移除旧工具留下的 App Nap 与磁盘映像验证覆盖', safe: true, status: 'pending' },
  { id: 'network_stack_optimize', name: '网络栈刷新', description: '刷新路由表与 ARP 缓存', safe: true, status: 'pending' },
  { id: 'disk_permissions_repair', name: '权限修复', description: '修复用户目录权限问题', safe: true, status: 'pending' },
  { id: 'spotlight_index_optimize', name: 'Spotlight 优化', description: '搜索缓慢时智能重建索引', safe: true, status: 'pending' },
  { id: 'spotlight_orphan_rules_cleanup', name: 'Spotlight 孤立规则', description: '清理已卸载应用的搜索规则', safe: true, status: 'pending' },
  { id: 'periodic_maintenance', name: '定期维护', description: '运行过期的 macOS 每日/每周/每月维护脚本', safe: true, status: 'pending' },
  { id: 'shared_file_list_repair', name: '共享文件列表', description: '修复损坏的 Finder 收藏与最近文档', safe: true, status: 'pending' },
  { id: 'disk_verify', name: '磁盘健康', description: '验证文件系统完整性', safe: true, status: 'pending' },
  { id: 'login_items_audit', name: '登录项审计', description: '审计损坏的登录项', safe: true, status: 'pending' },
  { id: 'quarantine_cleanup', name: '隔离数据库清理', description: '清除 Gatekeeper 下载跟踪历史', safe: true, status: 'pending' },
  { id: 'launch_agents_cleanup', name: 'Launch Agents 清理', description: '移除二进制已不存在的损坏 LaunchAgents', safe: true, status: 'pending' },
  { id: 'notification_cleanup', name: '通知清理', description: '清理旧通知以减少数据库膨胀', safe: true, status: 'pending' },
  { id: 'coreduet_cleanup', name: '使用数据清理', description: '清理旧的使用跟踪数据', safe: true, status: 'pending' },
]

export const OPTIMIZE_MOCK_SYSTEM_INFO: MoleOptimizeSystemInfo = {
  memory_used_gb: 9.4,
  memory_total_gb: 16,
  memory_used_percent: 58.8,
  disk_used_gb: 312,
  disk_total_gb: 512,
  disk_used_percent: 60.9,
  uptime_days: 3,
}

// 静态演示数据（has_bottleneck=true 用于展示瓶颈提示条）；
// 接后端后由 dry_run 返回真实诊断结果。
export const OPTIMIZE_MOCK_DIAGNOSTICS: MoleOptimizeDiagnostics = {
  has_bottleneck: true,
  sustained: [{ family: 'windowserver', label: 'WindowServer', avg_cpu: 68.4 }],
  detach_candidates: [],
  primary: {
    family: 'windowserver',
    label: 'WindowServer',
    avg_cpu: 68.4,
    note: 'WindowServer 持续占用较高 CPU，可能与外接显示器或桌面特效有关',
  },
}

export const OPTIMIZE_MOCK_DRY_RUN: MoleOptimizeResult = {
  mode: 'dry_run',
  collected_at: new Date().toISOString(),
  system_info: OPTIMIZE_MOCK_SYSTEM_INFO,
  tasks: OPTIMIZE_MOCK_TASKS,
  diagnostics: OPTIMIZE_MOCK_DIAGNOSTICS,
  summary: {
    total_tasks: OPTIMIZE_MOCK_TASKS.length,
    safe_count: OPTIMIZE_MOCK_TASKS.filter((t) => t.safe).length,
    would_apply_count: OPTIMIZE_MOCK_TASKS.length,
  },
}
