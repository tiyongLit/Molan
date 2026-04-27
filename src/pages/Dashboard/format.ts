// 仪表盘数据格式化与派生工具：快照字段 → 展示值。

/** 字节 → 人类可读：GB 档固定两位小数（柠檬托盘同格式，1.90 GB 而非 1.9） */
export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`
  return `${Math.max(0, Math.round(bytes / 1024))} KB`
}

/** 网速 MB/s → [数值, 单位]：≥1 MB/s 显示 MB/s，否则换算 KB/s */
export function formatRate(mbs: number): { value: string; unit: string } {
  if (mbs >= 1) return { value: mbs.toFixed(1), unit: 'MB/s' }
  return { value: String(Math.max(0, Math.round(mbs * 1024))), unit: 'KB/s' }
}

/** 网络历史降采样：后端 120 点 ring → sparkline 30 点 */
export function downsample(data: number[], target = 30): number[] {
  if (data.length <= target) return data
  const step = Math.floor(data.length / target)
  return data.filter((_, i) => i % step === 0).slice(-target)
}
