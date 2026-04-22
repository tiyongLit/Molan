import { useNativeIcon } from '@/hooks/useNativeIcon'

export interface AppIconProps {
  /** 应用名（用于 alt） */
  name: string
  /** 应用路径；存在时渲染原生图标（注册表订阅，未就绪显示骨架屏） */
  path?: string
  /** 图标边长（px），默认 28 */
  size?: number
  /** 直渲图标源（SVG data URI）：后端已解析好图标时直接渲染，
   * 跳过注册表（进程域图标场景：后端按 NSWorkspace runningApplications
   * 匹配后随快照下发）。优先级高于 path。 */
  iconSrc?: string | null
}

/**
 * 应用图标：原生图标优先（注册表订阅自动刷新），未命中显示加载骨架屏。
 *
 * 跨「卸载 / 更新 / 启动项」三个 tab 复用：
 * path 有值时由 useNativeIcon 自动 resolve + 订阅，解析完成即无缝换成真实图标。
 */
export function AppIcon({ name, path, size = 28, iconSrc }: AppIconProps) {
  const registryIcon = useNativeIcon(path ?? '')
  const nativeIcon = iconSrc ?? registryIcon

  if (nativeIcon) {
    return (
      <img
        src={nativeIcon}
        alt={name}
        className="rounded-md shrink-0 object-contain"
        style={{ width: size, height: size }}
      />
    )
  }

  // 加载骨架占位（柔和脉冲动画，非突兀色块）
  return (
    <div
      className="rounded-md shrink-0 animate-pulse"
      style={{
        width: size,
        height: size,
        background: 'rgba(255,255,255,0.08)',
      }}
    />
  )
}
