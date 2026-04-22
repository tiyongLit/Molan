/**
 * 平台 / 浏览器环境检测工具
 */

import type { DiskStatus } from '@/types/mole'

/**
 * 主磁盘选择（单一事实来源）：优先 mount === '/'，无匹配时回退首个。
 * Home / 托盘仪表盘 / Analyze 三处统一经此选盘，避免各自取盘口径不一致。
 */
export function pickPrimaryDisk(disks?: DiskStatus[]): DiskStatus | undefined {
  if (!disks?.length) return undefined
  return disks.find((d) => d.mount === '/') ?? disks[0]
}

/**
 * 磁盘可用空间统一口径（字节）：
 * 优先后端下发的 `free`（NSURLVolumeAvailableCapacityForImportantUsageKey，
 * 与 macOS「储存概述」及腾讯柠檬完全一致）；
 * 仅在旧快照缺失该字段时兜底 total - used。
 * 所有模块（Home/托盘/Analyze）必须经此函数取可用值，禁止各处自行 total - used。
 */
export function diskFreeBytes(disk?: DiskStatus): number {
  if (!disk) return 0
  if (typeof disk.free === 'number') return Math.max(0, disk.free)
  return Math.max(0, disk.total - disk.used)
}

/** 磁盘指标统一派生结果 */
export interface DiskMetrics {
  /** 可用空间（字节），经 diskFreeBytes() 统一口径 */
  free: number
  /** 已使用（字节），后端自洽三元组 */
  used: number
  /** 总容量（字节） */
  total: number
  /** 已使用百分比（0–100），后端下发 */
  usedPercent: number
  /** 显示名称（根卷 → Macintosh HD） */
  name: string
}

/**
 * 磁盘指标统一派生（纯函数）：
 * 从 DiskStatus 提取 free/used/total/usedPercent/name，保证三处（Home/托盘/Analyze）
 * 使用完全相同的数据口径。任何磁盘数值变更只需改此处。
 *
 * - free 统一经 diskFreeBytes()（NSURLVolumeAvailableCapacityForImportantUsageKey）
 * - used/total/usedPercent 直取后端自洽三元组，禁止本地推导
 */
export function deriveDiskMetrics(disk?: DiskStatus): DiskMetrics {
  return {
    free: diskFreeBytes(disk),
    used: disk?.used ?? 0,
    total: disk?.total ?? 0,
    usedPercent: disk?.used_percent ?? 0,
    name: disk?.mount === '/' ? 'Macintosh HD' : disk?.mount ?? '',
  }
}
