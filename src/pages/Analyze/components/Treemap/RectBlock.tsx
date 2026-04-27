import { memo, useMemo } from 'react'
import { Popover, Dropdown } from 'antd'
import { CheckOutlined } from '@ant-design/icons'
import { formatSize } from '@/utils/format'
import { MarqueeText } from '@/components/ui/MarqueeText'
import {
  CTX_MENU_CLASS,
  CTX_MENU_STYLE,
  type TreemapItem,
  type ContextMenuItem
} from '../../typings'

// ── 显示阈值 ──
const RECT_THRESHOLDS = {
  FULL: { width: 80, height: 60 },
  NAME_ONLY: { width: 45, height: 28 }
} as const

// ── 颜色常量（空间透镜风格：半透明磨砂方块让渐变透出 + 蓝色选中高亮，对齐全 app 蓝=选中语言） ──
const C = {
  border: 'rgba(255,255,255,0.16)',
  borderHover: 'rgba(96,165,250,0.55)',
  borderSelected: 'rgba(59,130,246,0.95)',
  bg: 'rgba(24,26,38,0.55)',
  bgLight: 'rgba(38,40,54,0.55)',
  bgSelected: 'rgba(30,42,72,0.75)',
  bgHover: 'rgba(20,22,32,0.65)',
  checkedOverlay: 'rgba(59,130,246,0.20)',
  checkedBadgeBg: '#3b82f6'
} as const

/** 图标渲染：原生图标（data URI）渲染 <img>，emoji 降级时渲染文字 */
function renderIcon(icon: string, size: number) {
  if (icon.startsWith('data:')) {
    return <img src={icon} alt="" style={{ width: size, height: size, flexShrink: 0 }} />
  }
  return (
    <span
      style={{
        width: size,
        height: size,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontSize: Math.max(size * 0.7, 10)
      }}
    >
      {icon}
    </span>
  )
}

interface RectBlockProps {
  item: TreemapItem
  isSelected: boolean
  isHovered: boolean
  isChecked: boolean
  onClick: () => void
  onDoubleClick: () => void
  onMouseEnter: () => void
  onMouseLeave: () => void
  buildContextMenu?: (item: TreemapItem) => { items: ContextMenuItem[] }
}

export const RectBlock = memo(function RectBlock({
  item,
  isSelected,
  isHovered,
  isChecked,
  onClick,
  onDoubleClick,
  onMouseEnter,
  onMouseLeave,
  buildContextMenu
}: RectBlockProps) {
  const { x, y, width, height } = item.rect

  const showFull = width >= RECT_THRESHOLDS.FULL.width && height >= RECT_THRESHOLDS.FULL.height
  const showNameOnly =
    width >= RECT_THRESHOLDS.NAME_ONLY.width && height >= RECT_THRESHOLDS.NAME_ONLY.height

  const borderColor = isSelected ? C.borderSelected : isHovered ? C.borderHover : C.border

  // checked 角标自适应
  const badgeSize = showFull ? 12 : showNameOnly ? 9 : 5
  const badgeFontSize = badgeSize <= 5 ? 5 : badgeSize <= 9 ? 6 : 8

  // ── 悬停气泡 — 对标 Lemon LMSpaceBubbleViewController 布局（190×48 紧凑两行）：
  // 图标 30px 垂直居中，名称（超宽跑马灯）在上、大小在下；不展示绝对路径
  //（完整路径可由左侧 EntryList 与面包屑导航获取）。 ──
  const popoverContent = useMemo(
    () => (
      <div className="flex items-center gap-2.5 min-w-[200px] max-w-[240px]">
        {renderIcon(item.icon, 30)}
        <div className="flex-1 min-w-0 flex flex-col gap-0.5">
          <MarqueeText
            text={item.name}
            className="text-sm font-medium text-white leading-tight"
          />
          <span className="text-xs text-white/60 font-mono">{formatSize(item.size)}</span>
        </div>
      </div>
    ),
    [item.name, item.size, item.icon]
  )

  const rectContent = useMemo(
    () => (
      <div
        onClick={onClick}
        onDoubleClick={onDoubleClick}
        onMouseEnter={onMouseEnter}
        onMouseLeave={onMouseLeave}
        style={{
          position: 'absolute',
          left: x,
          top: y,
          width,
          height,
          borderRadius: 4,
          cursor: 'pointer',
          overflow: 'hidden',
          border: `${isSelected ? 2 : 1}px solid ${borderColor}`,
          background: isSelected
            ? `linear-gradient(145deg, ${C.bgSelected}, ${C.bgHover})`
            : `linear-gradient(145deg, ${C.bg}, ${C.bgLight})`,
          backdropFilter: 'blur(6px)',
          WebkitBackdropFilter: 'blur(6px)',
          boxShadow: isSelected
            ? '0 4px 14px rgba(0,0,0,0.35), 0 0 14px rgba(59,130,246,0.32), inset 0 1px 0 rgba(255,255,255,0.08)'
            : isHovered
              ? '0 2px 8px rgba(0,0,0,0.28), 0 0 8px rgba(59,130,246,0.18), inset 0 1px 0 rgba(255,255,255,0.06)'
              : '0 1px 4px rgba(0,0,0,0.20), inset 0 1px 0 rgba(255,255,255,0.05)',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 3,
          transition: 'border-color 0.15s ease, box-shadow 0.15s ease, background 0.15s ease'
        }}
      >
        {isChecked && (
          <div
            style={{
              position: 'absolute',
              inset: 0,
              background: C.checkedOverlay,
              pointerEvents: 'none',
              zIndex: 1
            }}
          />
        )}
        {isChecked && (
          <div
            style={{
              position: 'absolute',
              top: 3,
              right: 3,
              width: badgeSize,
              height: badgeSize,
              borderRadius: badgeSize <= 8 ? '50%' : 3,
              background: C.checkedBadgeBg,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              zIndex: 2,
              boxShadow: '0 1px 3px rgba(0,0,0,0.25)'
            }}
          >
            {badgeSize >= 12 && (
              <CheckOutlined style={{ color: '#fff', fontSize: badgeFontSize, lineHeight: 1 }} />
            )}
          </div>
        )}
        {showFull && renderIcon(item.icon, Math.min(width * 0.18, 28))}
        {showNameOnly && (
          <span
            style={{
              color: isSelected ? '#FFFFFF' : 'rgba(255,255,255,0.85)',
              fontSize: Math.max(Math.min(width * 0.065, 13), 10),
              textAlign: 'center',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              maxWidth: Math.max(width - 12, 0),
              lineHeight: '1.4',
              fontWeight: isSelected ? 600 : 500
            }}
          >
            {item.name}
          </span>
        )}
        {showFull && (
          <span
            style={{
              color: 'rgba(255,255,255,0.55)',
              fontSize: Math.max(Math.min(width * 0.055, 12), 9),
              lineHeight: '1.4',
              fontFamily: 'ui-monospace, SFMono-Regular, monospace'
            }}
          >
            {formatSize(item.size)}
          </span>
        )}
      </div>
    ),
    [
      x,
      y,
      width,
      height,
      isSelected,
      isHovered,
      isChecked,
      badgeSize,
      badgeFontSize,
      showFull,
      showNameOnly,
      item.icon,
      item.name,
      item.size,
      onClick,
      onDoubleClick,
      onMouseEnter,
      onMouseLeave,
      borderColor
    ]
  )

  const wrapped = (
    <Popover
      content={popoverContent}
      placement="right"
      trigger="hover"
      arrow={{ pointAtCenter: true }}
    >
      {rectContent}
    </Popover>
  )

  if (buildContextMenu) {
    const menuItems = buildContextMenu(item).items
    return (
      <Dropdown
        menu={{ items: menuItems, className: CTX_MENU_CLASS, style: CTX_MENU_STYLE }}
        trigger={['contextMenu']}
      >
        {wrapped}
      </Dropdown>
    )
  }
  return wrapped
})
