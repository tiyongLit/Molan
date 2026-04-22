import { nativeIconRegistry } from '@/utils/nativeIconRegistry'
import type { IconSrc } from '@/utils/nativeIconRegistry'
import { resolveStaticIcon, type IconInput } from '@/utils/staticIconMap'

export interface NativeIconProps extends IconInput {
  /** 图标边长（px），默认 28 */
  size?: number
  /** 额外 className（圆角、shrink-0 等，与业务样式解耦） */
  className?: string
  /** 优先使用的图标源（data URI / base64）：后端随帧下发时直接渲染，
   * 跳过注册表缓存。用于进程域（metrics_process 随帧下发的 icon 字段）。 */
  iconSrc?: string | null
  /** 自定义 alt（默认 name） */
  alt?: string
}

/**
 * 原生图标：注册表命中显示 SVG，未命中降级到 emoji（resolveStaticIcon）。
 *
 * 设计要点：
 *   - 优先用 prop `iconSrc`（后端随帧下发的进程图标等场景）
 *   - 否则 `nativeIconRegistry.get(path)`（content-addressed SVG）
 *   - 都没有时显示 emoji（同步、无异步闪烁，对齐 Lemon 图标加载语义）
 *
 * 跨 Uninstall / Dashboard / Analyze / Home 统一渲染口径。
 *
 * 注：调用方应确保已对 path 调过 `nativeIconRegistry.resolve` / `resolveIdle`，
 * 否则 NativeIcon 只渲染 emoji。列表场景用 useNativeIconMap 订阅变更即可。
 */
export function NativeIcon({
  path,
  name,
  isDir,
  size = 28,
  className,
  iconSrc,
  alt,
}: NativeIconProps) {
  const resolved: IconSrc = iconSrc ?? nativeIconRegistry.get(path)

  if (resolved) {
    return (
      <img
        src={resolved}
        alt={alt ?? name}
        className={`shrink-0 object-contain ${className ?? ''}`}
        style={{ width: size, height: size }}
        draggable={false}
      />
    )
  }

  // Emoji 兜底：与显示尺寸同 font-size，行内垂直居中
  const emoji = resolveStaticIcon({ path, name, isDir })
  return (
    <span
      role="img"
      aria-label={alt ?? name}
      className={`inline-flex shrink-0 items-center justify-center ${className ?? ''}`}
      style={{ width: size, height: size, fontSize: size * 0.75, lineHeight: 1 }}
    >
      {emoji}
    </span>
  )
}
