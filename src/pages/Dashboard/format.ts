// 仪表盘数据格式化与派生工具：快照字段 → 展示值。

/** 字节 → 人类可读：GB 档固定两位小数（柠檬托盘同格式，1.90 GB 而非 1.9） */
export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`
  return `${Math.max(0, Math.round(bytes / 1024))} KB`
}
