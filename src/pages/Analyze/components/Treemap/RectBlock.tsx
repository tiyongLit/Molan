import { memo, useMemo } from 'react'
import { CheckOutlined } from '@ant-design/icons'
import { formatSize } from '@/utils/format'
import type { TreemapItem } from '../../typings'

// ── 显示阈值 ──
const RECT_THRESHOLDS = {
  FULL: { width: 80, height: 60 },
  NAME_ONLY: { width: 45, height: 28 }
} as const

// ── 颜色常量（空间透镜风格：半透明磨砂方块让渐变透出 + 蓝色选中高亮，对齐全 app 蓝=选中语言） ──
const C = {
  border: 'rgba(255,255,255,0.16)',
  borderSelected: 'rgba(59,130,246,0.95)',
  bg: 'rgba(24,26,38,0.55)',
  bgLight: 'rgba(38,40,54,0.55)',
  bgSelected: 'rgba(30,42,72,0.75)',
  bgHover: 'rgba(20,22,32,0.65)',
  checkedOverlay: 'rgba(59,130,246,0.20)',
  checkedBadgeBg: '#3b82f6'
} as const

/** 图标渲染：原生图标（data URI）渲染 <img>（150ms 淡入），emoji 降级时渲染文字 */
function renderIcon(icon: string, size: number) {
  if (icon.startsWith('data:')) {
    return <img src={icon} alt="" className="analyze-icon-fade" style={{ width: size, height: size, flexShrink: 0 }} />
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
  isChecked: boolean
}

/**
 * Treemap 单个方格 — 纯展示组件。
 *
 * **架构原则**：
 * - 只接收纯数据 props（item / isSelected / isChecked），不接收任何回调
 * - 交互（click / dblclick / contextmenu）由 Treemap 容器层事件委托统一处理
 * - 悬停气泡由全局单例 AnalyzeHoverCard 接管，不再包裹 antd Popover
 * - 右键菜单由全局单例 AnalyzeContextMenu 接管，不再包裹 antd Dropdown
 *
 * **hover 视觉**：纯 CSS `:hover` 伪类 + transition 实现，不依赖 JS state。
 * 原先的 isHovered prop 已删除，避免每次悬停都触发重渲染。
 *
 * **data 属性**：写入 data-path / data-name / data-protected，供事件委托层识别目标。
 */
export const RectBlock = memo(function RectBlock({
  item,
  isSelected,
  isChecked
}: RectBlockProps) {
  const { x, y, width, height } = item.rect

  const showFull = width >= RECT_THRESHOLDS.FULL.width && height >= RECT_THRESHOLDS.FULL.height
  const showNameOnly =
    width >= RECT_THRESHOLDS.NAME_ONLY.width && height >= RECT_THRESHOLDS.NAME_ONLY.height

  // checked 角标自适应
  const badgeSize = showFull ? 12 : showNameOnly ? 9 : 5
  const badgeFontSize = badgeSize <= 5 ? 5 : badgeSize <= 9 ? 6 : 8

  // 样式计算（useMemo 避免每次渲染重建对象）
  const blockStyle = useMemo(
    () => ({
      position: 'absolute' as const,
      left: x,
      top: y,
      width,
      height,
      borderRadius: 4,
      cursor: 'pointer',
      overflow: 'hidden',
      border: `${isSelected ? 2 : 1}px solid ${isSelected ? C.borderSelected : C.border}`,
      background: isSelected
        ? `linear-gradient(145deg, ${C.bgSelected}, ${C.bgHover})`
        : `linear-gradient(145deg, ${C.bg}, ${C.bgLight})`,
      backdropFilter: 'blur(6px)',
      WebkitBackdropFilter: 'blur(6px)',
      boxShadow: isSelected
        ? '0 4px 14px rgba(0,0,0,0.35), 0 0 14px rgba(59,130,246,0.32), inset 0 1px 0 rgba(255,255,255,0.08)'
        : '0 1px 4px rgba(0,0,0,0.20), inset 0 1px 0 rgba(255,255,255,0.05)',
      display: 'flex',
      flexDirection: 'column' as const,
      alignItems: 'center',
      justifyContent: 'center',
      gap: 3,
      transition: 'border-color 0.15s ease, box-shadow 0.15s ease, background 0.15s ease'
    }),
    [x, y, width, height, isSelected]
  )

  return (
    <div
      data-path={item.path}
      data-name={item.name}
      data-protected={item.protected ? 'true' : 'false'}
      className="analyze-rect-block select-none"
      style={blockStyle}
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
  )
})
