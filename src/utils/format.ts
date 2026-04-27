import { SIZE_BASE } from '@/constants/shared'

export function formatSize(bytes: number): string {
  // 统一使用二进制 1024 进制（KB/MB/GB），与后端 bytes_to_human 一致
  if (bytes === 0) return '0 B'
  const k = SIZE_BASE
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`
}

export function formatSizeSimple(bytes: number): string {
  if (bytes === 0) return '0 B'
  const k = SIZE_BASE
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  if (i === 0) return `${bytes} B`
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`
}

export function formatPercent(value: number, decimals = 1): string {
  return `${value.toFixed(decimals)}%`
}

export function formatNetworkSpeed(mbs: number): string {
  if (mbs < 0.001) return '0 B/s'
  if (mbs < 1) return `${(mbs * 1024).toFixed(1)} KB/s`
  return `${mbs.toFixed(2)} MB/s`
}

export function getHealthColor(score: number): string {
  if (score >= 80) return '#22c55e'
  if (score >= 60) return '#eab308'
  if (score >= 40) return '#f97316'
  return '#ef4444'
}

export function getHealthLabel(score: number): string {
  if (score >= 80) return '优秀'
  if (score >= 60) return '良好'
  if (score >= 40) return '一般'
  return '较差'
}

export function getUsageColor(percent: number): string {
  if (percent < 50) return '#22c55e'
  if (percent < 80) return '#eab308'
  return '#ef4444'
}
export function humanDiskSize(bytes: number): string {
  const k = SIZE_BASE
  if (bytes >= k * k * k * k) return `${(bytes / (k * k * k * k)).toFixed(2)} TB`
  if (bytes >= k * k * k) return `${(bytes / (k * k * k)).toFixed(2)} GB`
  if (bytes >= k * k) return `${(bytes / (k * k)).toFixed(1)} MB`
  if (bytes >= k) return `${(bytes / k).toFixed(1)} KB`
  return `${bytes} B`
}
export function formatNumber(n: number): string {
  if (n < 1_000) return `${n}`
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  return `${(n / 1_000_000_000).toFixed(1)}G`
}
