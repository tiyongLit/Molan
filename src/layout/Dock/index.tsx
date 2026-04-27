import { useRef, useEffect } from 'react'
import { useMotionValue, useMotionValueEvent } from 'motion/react'
import type { ReactNode } from 'react'
import { Settings } from 'lucide-react'
import { DockItem } from './DockItem'
import './dock.scss'

// ── Types ──

export interface NavItem {
  id: string
  label: string
  icon: ReactNode
  accent?: string
}

// ── Constants ──
// 弹簧调优：临界阻尼 = 2√(m·k) ≈ 11；当前 damping=10 → 阻尼比 ≈ 0.87
// 略欠阻尼，快速到位带一丝微妙回弹，贴近 macOS Dock 手感
const DOCK_SPRING = { mass: 0.1, stiffness: 350, damping: 10 }
const DOCK_MAGNIFICATION = 52
const DOCK_BASE_SIZE = 36
const DOCK_SIGMA = 56

// ── FloatingDock ──

interface FloatingDockProps {
  items: NavItem[]
  activeId: string
  onNavigate: (id: string) => void
  showSettings?: boolean
}

function FloatingDock({ items, activeId, onNavigate, showSettings = true }: FloatingDockProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const mouseY = useMotionValue(Infinity)

  // 集中式高斯放大：每个 DockItem 一个独立 MotionValue，
  // 在事件处理器中批量计算（脱离 React 渲染路径），彻底消除 layout thrashing
  const sizeTargets = useRef(
    Array.from({ length: items.length + (showSettings ? 1 : 0) }, () => DOCK_BASE_SIZE)
  )
  const sizeValues = useRef(
    Array.from({ length: items.length + (showSettings ? 1 : 0) }, () =>
      useMotionValue(DOCK_BASE_SIZE)
    )
  )

  // 缓存各 DockItem 中心 Y 坐标，仅在挂载 / 窗口 resize 时刷新
  const centerYCache = useRef<number[]>([])

  const recalcPositions = () => {
    if (!containerRef.current) return
    centerYCache.current = Array.from(
      containerRef.current.querySelectorAll<HTMLElement>('[data-dock-id]')
    ).map((el) => {
      const r = el.getBoundingClientRect()
      return r.y + r.height / 2
    })
  }

  useEffect(() => {
    recalcPositions()
    window.addEventListener('resize', recalcPositions)
    return () => window.removeEventListener('resize', recalcPositions)
  }, [])

  // 鼠标移动时一次性计算所有 item 的目标尺寸（事件处理器中执行，不触发 reflow 链）
  useMotionValueEvent(mouseY, 'change', (pageY: number) => {
    const centers = centerYCache.current
    for (let i = 0; i < sizeValues.current.length; i++) {
      const cy = centers[i]
      if (cy === undefined) continue
      const dist = Math.abs(pageY - cy)
      const t = dist / DOCK_SIGMA
      const target =
        DOCK_BASE_SIZE + (DOCK_MAGNIFICATION - DOCK_BASE_SIZE) * Math.exp(-0.5 * t * t)
      sizeTargets.current[i] = target
      sizeValues.current[i].set(target)
    }
  })

  const handleMouseLeave = () => {
    mouseY.set(Infinity)
    centerYCache.current = []
    for (let i = 0; i < sizeValues.current.length; i++) {
      sizeTargets.current[i] = DOCK_BASE_SIZE
      sizeValues.current[i].set(DOCK_BASE_SIZE)
    }
  }

  const allItems = showSettings
    ? [
        ...items,
        null as unknown as NavItem, // 分隔线占位
        { id: 'settings', label: 'Settings', icon: <Settings size={18} />, accent: '#89b4fa' }
      ]
    : items

  return (
    <div className="relative flex flex-col items-start select-none" ref={containerRef}>
      <div
        className="dock-container"
        style={{ transform: 'translate3d(0,0,0.01px)' }}
        onMouseMove={({ pageY }) => mouseY.set(pageY)}
        onMouseLeave={handleMouseLeave}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {allItems.map((item, i) => {
          // 分隔线
          if (item === null) {
            return (
              <div
                key="sep"
                style={{
                  width: 36,
                  height: 1,
                  background: 'rgba(69, 71, 90, 0.5)',
                  margin: '2px 0',
                  flexShrink: 0
                }}
              />
            )
          }
          return (
            <DockItem
              key={item.id}
              item={item}
              active={activeId === item.id}
              onClick={() => onNavigate(item.id)}
              dockId={item.id}
              sizeMotion={sizeValues.current[i]}
              spring={DOCK_SPRING}
              baseItemSize={DOCK_BASE_SIZE}
            />
          )
        })}
      </div>
    </div>
  )
}

export default FloatingDock
