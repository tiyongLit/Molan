/**
 * 与 CLI sizeColorForPercent 一致的四级颜色编码
 * 公开 API（getColorLevel / SIZE_COLOR_CLASS）被 EntryRow / TopBar / Top 20 行共用；
 * ProgressBar 组件已下线（其使用的 BAR_COLOR_CLASS 保留以备复用）。
 */

export type ColorLevel = 'red' | 'yellow' | 'blue' | 'gray'

export function getColorLevel(percent: number): ColorLevel {
  if (percent >= 50) return 'red'
  if (percent >= 20) return 'yellow'
  if (percent >= 5) return 'blue'
  return 'gray'
}

/** EntryRow 中文字颜色 */
export const SIZE_COLOR_CLASS: Record<ColorLevel, string> = {
  red: 'text-red-400',
  yellow: 'text-yellow-500',
  blue: 'text-blue-400',
  gray: 'text-[var(--text-tertiary)]'
}

/** ProgressBar 中背景颜色 */
export const BAR_COLOR_CLASS: Record<ColorLevel, string> = {
  red: 'bg-red-500',
  yellow: 'bg-yellow-500',
  blue: 'bg-blue-500',
  gray: 'bg-gray-400'
}
