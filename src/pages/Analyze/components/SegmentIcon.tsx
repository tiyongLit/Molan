import { useIcon } from '@/hooks/useIcon'

const ICON_SIZE = 14

/** 面包屑段图标 — 原生系统图标（NSWorkspace data URI）优先，emoji 降级 */
export function SegmentIcon({ path }: { path: string }) {
  const src = useIcon(path)

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
