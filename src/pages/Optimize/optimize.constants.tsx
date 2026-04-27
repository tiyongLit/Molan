import type { ReactNode, CSSProperties } from 'react'
import {
  GlobalOutlined,
  HddOutlined,
  SearchOutlined,
  AppstoreOutlined,
  SafetyCertificateOutlined,
  DeleteOutlined,
} from '@ant-design/icons'

// ============================================================
// 任务分组（对齐 Clean 的 CATEGORY_GROUPS 模式）
// actionIds 对齐 Mole lib/optimize/catalog.sh 的 21 个安全任务
// ============================================================
export interface OptimizeGroupDef {
  id: string
  title: string
  actionIds: readonly string[]
}

export const TASK_GROUPS: OptimizeGroupDef[] = [
  { id: 'network', title: '网络与系统', actionIds: ['system_maintenance', 'network_optimization', 'network_stack_optimize', 'prevent_network_dsstore'] },
  { id: 'disk', title: '磁盘与权限', actionIds: ['disk_verify', 'disk_permissions_repair', 'periodic_maintenance'] },
  { id: 'spotlight', title: 'Spotlight', actionIds: ['spotlight_index_optimize', 'spotlight_orphan_rules_cleanup'] },
  { id: 'app_db', title: '应用与数据库', actionIds: ['sqlite_vacuum', 'saved_state_cleanup', 'fix_broken_configs', 'launch_services_rebuild', 'cache_refresh', 'shared_file_list_repair'] },
  { id: 'startup', title: '启动与安全', actionIds: ['login_items_audit', 'launch_agents_cleanup', 'quarantine_cleanup', 'legacy_overrides_audit'] },
  { id: 'data', title: '数据清理', actionIds: ['notification_cleanup', 'coreduet_cleanup'] },
]

// 分组图标（@ant-design/icons，统一白色透明，对齐 Clean 风格）
export const groupIconMap: Record<string, ReactNode> = {
  network: <GlobalOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  disk: <HddOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  spotlight: <SearchOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  app_db: <AppstoreOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  startup: <SafetyCertificateOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  data: <DeleteOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
}

// ============================================================
// 执行阶段任务运行时状态
// ============================================================
export interface TaskRuntime {
  action: string
  name: string
  status: 'pending' | 'running' | 'success' | 'skipped' | 'failed'
  note?: string
}

// 主 CTA 按钮统一尺寸（立即分析 / 取消 / 开始优化）
export const OPTIMIZE_CTA_STYLE: CSSProperties = {
  width: 206,
  height: 60,
  fontSize: 28,
  borderRadius: 2,
}
