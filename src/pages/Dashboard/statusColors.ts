// Dashboard 卡片状态颜色判断（三级：正常/警告/严重）
//
// 统一的阈值模型，所有卡片共享同一套颜色语义：
// - 正常：主题默认色（textPrimary）
// - 警告：黄色（warn）
// - 严重：红色（critical）

import { dashTheme as t } from './theme'

export type StatusLevel = 'normal' | 'warn' | 'critical'

/** 根据状态级别返回对应颜色 */
export function statusColor(level: StatusLevel): string {
  switch (level) {
    case 'warn':
      return t.warn
    case 'critical':
      return t.critical
    default:
      return t.textPrimary
  }
}

/** CPU/GPU 使用率状态（越高越严重） */
export function usageStatus(usage: number): StatusLevel {
  if (usage >= 90) return 'critical'
  if (usage >= 70) return 'warn'
  return 'normal'
}

/** 内存状态（可用百分比，越低越严重） */
export function memoryStatus(availablePercent: number): StatusLevel {
  if (availablePercent < 10) return 'critical'
  if (availablePercent < 30) return 'warn'
  return 'normal'
}

/** 磁盘状态（可用百分比，越低越严重） */
export function diskStatus(availablePercent: number): StatusLevel {
  if (availablePercent < 10) return 'critical'
  if (availablePercent < 20) return 'warn'
  return 'normal'
}

/** 风扇状态（RPM，越高越严重） */
export function fanStatus(rpm: number): StatusLevel {
  if (rpm > 7000) return 'critical'
  if (rpm > 4500) return 'warn'
  return 'normal'
}

/** CPU 温度状态（°C，越高越严重） */
export function tempStatus(temp: number): StatusLevel {
  if (temp >= 85) return 'critical'
  if (temp >= 65) return 'warn'
  return 'normal'
}
