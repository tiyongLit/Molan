import { iconService } from '@/utils/iconService'

export interface AppIconProps {
  /** 应用名（用于 alt） */
  name: string
  /** 应用路径；存在时优先渲染原生图标（需已预加载） */
  path?: string
  /** 图标边长（px），默认 28 */
  size?: number
  /** 直渲图标源（data URI / base64）：后端已解析好图标时直接渲染，
   * 跳过 iconService 缓存管线（进程域图标场景：后端按 NSWorkspace
   * runningApplications 匹配后随快照下发）。优先级高于 path。 */
  iconSrc?: string | null
}

/**
 * 应用图标：原生图标优先（命中缓存），未命中显示加载骨架（柔和脉冲动画）。
 *
 * 跨「卸载 / 更新 / 启动项」三个 tab 复用。
 * 不再显示首字母色块——配合 index.tsx 的延迟渲染策略，首次渲染时图标已预加载完成。
 */
export function AppIcon({ name, path, size = 28, iconSrc }: AppIconProps) {
  const nativeIcon = iconSrc ?? (path ? iconService.getCachedSync(path) : null)

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
