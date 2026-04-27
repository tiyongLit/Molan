import type { ReactNode, CSSProperties } from 'react'
import {
  DatabaseOutlined,
  AppstoreOutlined,
  GlobalOutlined,
  CodeOutlined,
  UsbOutlined,
  FileSearchOutlined,
} from '@ant-design/icons'
import type { MoleCleanItem } from '@/types/mole'

// ============================================================
// 分类分组（UI 展示用）
// ============================================================
interface CategoryGroupDef {
  id: string
  title: string
  categoryIds: readonly string[]
}

export const CATEGORY_GROUPS: CategoryGroupDef[] = [
  { id: 'system', title: '系统缓存', categoryIds: ['system_caches', 'time_machine', 'apple_silicon', 'system_data_clues', 'logs'] },
  { id: 'app', title: '应用缓存', categoryIds: ['user_cache', 'app_caches', 'app_support_logs', 'applications', 'orphaned_data', 'project_artifacts', 'trash'] },
  { id: 'browser_cloud', title: '浏览器与云', categoryIds: ['browser_cache', 'cloud_storage', 'office_cache'] },
  { id: 'dev', title: '开发者', categoryIds: ['dev_tools', 'virtualization'] },
  { id: 'device', title: '设备固件', categoryIds: ['device_firmware'] },
  { id: 'large', title: '大文件', categoryIds: ['large_files'] },
]

// 分类图标映射（@ant-design/icons，CleanMyMac 风格：统一白色透明）
export const categoryIconMap: Record<string, ReactNode> = {
  system: <DatabaseOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  app: <AppstoreOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  browser_cloud: <GlobalOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  dev: <CodeOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  device: <UsbOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
  large: <FileSearchOutlined style={{ fontSize: 15, color: 'rgba(255,255,255,0.7)' }} />,
}

/** 分类分组 + 派生统计数据（groups useMemo 产物） */
export interface CleanGroupData {
  id: string
  title: string
  categoryIds: readonly string[]
  items: MoleCleanItem[]
  totalSize: number
  selectedSize: number
  selectedCount: number
  itemCount: number
}

/** 主 CTA 按钮统一尺寸（立即扫描/取消/立即清理） */
export const PRIMARY_CTA_STYLE: CSSProperties = {
  width: 206,
  height: 60,
  fontSize: 28,
  borderRadius: 2,
}
