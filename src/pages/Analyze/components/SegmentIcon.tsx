import { useNativeIcon } from '@/hooks/useNativeIcon'

const ICON_SIZE = 14

/** 面包屑段图标 — 原生系统图标（注册表 SVG data URI）优先，emoji 降级 */
export function SegmentIcon({ path }: { path: string }) {
  const src = useNativeIcon(path)

  if (src && src.startsWith('data:')) {
    return (
      <img
        src={src}
        alt=""
        draggable={false}
        className="shrink-0 object-contain"
        style={{ width: ICON_SIZE, height: ICON_SIZE }}
      />
    )
  }

  return (
    <span className="shrink-0 leading-none select-none" style={{ fontSize: ICON_SIZE }}>
      📁
    </span>
  )
}
