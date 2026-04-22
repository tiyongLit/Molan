import { SIZE_BASE } from '@/constants/shared'

export function formatSize(bytes: number): string {
  // 统一使用二进制 1024 进制（KB/MB/GB），与后端 bytes_to_human 一致
  if (bytes === 0) return '0 B'
  const k = SIZE_BASE
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`
}

export function humanDiskSize(bytes: number): string {
  const k = SIZE_BASE
  if (bytes >= k * k * k * k) return `${(bytes / (k * k * k * k)).toFixed(2)} TB`
  if (bytes >= k * k * k) return `${(bytes / (k * k * k)).toFixed(2)} GB`
  if (bytes >= k * k) return `${(bytes / (k * k)).toFixed(1)} MB`
  if (bytes >= k) return `${(bytes / k).toFixed(1)} KB`
  return `${bytes} B`
}
