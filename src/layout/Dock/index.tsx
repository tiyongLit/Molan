import { Fragment, useRef, type KeyboardEvent } from 'react'
import { useMotionValue } from 'motion/react'
import { Settings } from 'lucide-react'
import { DockItem, type NavItem } from './DockItem'
import './dock.scss'
import { useI18n } from '@/i18n'

export type { NavItem }

// ── 调参 ──

// 基准：item 36px + 间距 6px → 相邻中心距 42px
// SIGMA = 36：邻位放大 ≈ 51%、次邻 ≈ 7%，形成"光标处拉近"而非整条膨胀
// 临界阻尼 = 2√(m·k) = 2√(0.1×450) ≈ 13.4 → damping 取临界值：无回弹、紧致吸附感
const DOCK_SPRING = { mass: 0.1, stiffness: 450, damping: 13.4 }
const DOCK_MAGNIFICATION = 56
const DOCK_BASE_SIZE = 36
const DOCK_SIGMA = 36

// ── FloatingDock ──

interface FloatingDockProps {
  items: NavItem[]
  activeId: string
  onNavigate: (id: string) => void
  /** 鼠标悬停某个导航项时触发（用于路由 chunk 预加载） */
  onItemHover?: (id: string) => void
  showSettings?: boolean
}

function FloatingDock({ items, activeId, onNavigate, onItemHover, showSettings = true }: FloatingDockProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { t } = useI18n()

  // 唯一的动效状态源：指针 Y（视口坐标；Infinity = 指针已离开）。
  // 各 DockItem 订阅它并自行测量中心、计算高斯放大，
  // 无集中索引、无中心缓存数组，分隔线/动态增删项都不会再错位
  const mouseY = useMotionValue(Infinity)

  // settings 项通过 dividerBefore 生成分隔线：分隔线渲染为独立兄弟节点，不占放大索引
  const presentItems: NavItem[] = showSettings
    ? [
        ...items,
        {
          id: 'settings',
          labelKey: 'nav.settings',
          icon: <Settings size={18} />,
          accent: '#89b4fa',
          dividerBefore: true
        }
      ]
    : items

  // 键盘导航（垂直 roving tabindex）：方向键移动焦点并切换页面
  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const focusables = Array.from(
      containerRef.current?.querySelectorAll<HTMLElement>('[data-dock-item]') ?? []
    )
    if (!focusables.length) return

    const focusedIndex = focusables.indexOf(document.activeElement as HTMLElement)
    const start =
      focusedIndex !== -1
        ? focusedIndex
        : Math.max(
            presentItems.findIndex((it) => it.id === activeId),
            0
          )

    let next: number
    switch (e.key) {
      case 'ArrowDown':
        next = Math.min(start + 1, focusables.length - 1)
        break
      case 'ArrowUp':
        next = Math.max(start - 1, 0)
        break
      case 'Home':
        next = 0
        break
      case 'End':
        next = focusables.length - 1
        break
      default:
        return
    }

    e.preventDefault()
    focusables[next]?.focus()
    const target = presentItems[next]
    if (target) onNavigate(target.id)
  }

  return (
    <div className="relative flex flex-col items-start select-none" ref={containerRef}>
      <div
        className="dock-container"
        style={{ transform: 'translate3d(0,0,0.01px)' }}
        role="toolbar"
        aria-orientation="vertical"
        aria-label={t('nav.ariaLabel')}
        onMouseMove={({ clientY }) => mouseY.set(clientY)}
        onMouseLeave={() => mouseY.set(Infinity)}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        {presentItems.map((item) => (
          <Fragment key={item.id}>
            {item.dividerBefore && <div className="dock-divider" aria-hidden="true" />}
            <DockItem
              item={item}
              active={activeId === item.id}
              onClick={() => onNavigate(item.id)}
              onHover={onItemHover}
              mouseY={mouseY}
              spring={DOCK_SPRING}
              baseItemSize={DOCK_BASE_SIZE}
              magnification={DOCK_MAGNIFICATION}
              sigma={DOCK_SIGMA}
            />
          </Fragment>
        ))}
      </div>
    </div>
  )
}

export default FloatingDock
