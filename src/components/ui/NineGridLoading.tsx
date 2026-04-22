import classnames from 'classnames'
import type { CSSProperties } from 'react'
import './NineGridLoading.css'

export interface NineGridLoadingProps {
  /** 是否播放脉冲 loading 动画（动画未开启时仅静态展示 9 宫格） */
  loading?: boolean
  /**
   * 主色。三档透明度（100% / 55% / 25%）由该色派生。
   * 默认白色（Clean）；Optimize 传入橙色 #fb923c。
   */
  color?: string
}

/**
 * 为 hex 色值追加 alpha 通道（#rrggbb → #rrggbbaa）。
 * 兼容不支持 color-mix() 的旧版 WebKit（Tauri WKWebView on macOS 12）。
 */
function withAlpha(hex: string, alpha: number): string {
  const a = Math.round(alpha * 255).toString(16).padStart(2, '0')
  return hex.length === 7 ? `${hex}${a}` : hex.length === 9 ? `${hex.slice(0, 7)}${a}` : hex
}

/**
 * 九宫格脉冲加载装饰（Clean 白 / Optimize 橙共用）。
 *
 * 原 CleanLoading 与 OptimizeLoading 结构完全一致、仅配色不同，
 * 故合并为本组件：通过 `color` 参数注入主题色，派生三档透明度格块。
 *
 * @example
 * <NineGridLoading loading={isScanning} />                 // 白色（默认）
 * <NineGridLoading loading={isExecuting} color="#fb923c" /> // 橙色
 */

export function NineGridLoading({ loading = false, color = '#ffffff' }: NineGridLoadingProps) {
  const style = {
    '--nine-grid-color': color,
    '--nine-grid-color-55': withAlpha(color, 0.55),
    '--nine-grid-color-25': withAlpha(color, 0.25),
  } as CSSProperties

  return (
    <div className="nine-grid-loading" style={style}>
      <div className={classnames('nine-grid-loading__grid', { animation: loading })}>
        {Array.from({ length: 9 }).map((_, i) => (
          <span key={i} />
        ))}
      </div>
    </div>
  )
}