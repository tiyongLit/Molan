import type { ReactNode, CSSProperties } from 'react'
import {
  GlobalOutlined,
  HddOutlined,
  SearchOutlined,
  AppstoreOutlined,
  SafetyCertificateOutlined,
  DeleteOutlined,
} from '@ant-design/icons'
import type { TranslationKey } from '@/i18n'

// ============================================================
// 任务分组（对齐 Clean 的 CATEGORY_GROUPS 模式）
// actionIds 对齐 Mole lib/optimize/catalog.sh 的 21 个安全任务
// ============================================================
export interface OptimizeGroupDef {
  id: string
  /** 分组标题 i18n key（组件内用 t(titleKey) 解析） */
  titleKey: TranslationKey
  actionIds: readonly string[]
}

export const TASK_GROUPS: OptimizeGroupDef[] = [
  { id: 'network', titleKey: 'optimize.group.network', actionIds: ['system_maintenance', 'network_optimization', 'network_stack_optimize', 'prevent_network_dsstore'] },
  { id: 'disk', titleKey: 'optimize.group.disk', actionIds: ['disk_verify', 'disk_permissions_repair', 'periodic_maintenance'] },
  { id: 'spotlight', titleKey: 'optimize.group.spotlight', actionIds: ['spotlight_index_optimize', 'spotlight_orphan_rules_cleanup'] },
  { id: 'app_db', titleKey: 'optimize.group.appDb', actionIds: ['sqlite_vacuum', 'saved_state_cleanup', 'fix_broken_configs', 'launch_services_rebuild', 'cache_refresh', 'shared_file_list_repair'] },
  { id: 'startup', titleKey: 'optimize.group.startup', actionIds: ['login_items_audit', 'launch_agents_cleanup', 'quarantine_cleanup', 'legacy_overrides_audit'] },
  { id: 'data', titleKey: 'optimize.group.data', actionIds: ['notification_cleanup', 'coreduet_cleanup'] },
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
// 不设固定 width，由文案内容自适应宽度（避免「开始优化（N）」文案变长时溢出）
export const OPTIMIZE_CTA_STYLE: CSSProperties = {
  minWidth: 206,
  height: 60,
  fontSize: 28,
  borderRadius: 2,
  padding: '0 24px',
}
