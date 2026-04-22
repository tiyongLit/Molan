import { useLayoutEffect, useRef, type ReactNode } from 'react'
import {
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  type MotionValue,
  type SpringOptions
} from 'motion/react'
import { Popover } from 'antd'
import { useI18n, type TranslationKey } from '@/i18n'

export interface NavItem {
  id: string
  labelKey: TranslationKey
  icon: ReactNode
  accent?: string
  /** 在该项上方渲染分隔线（渲染为独立兄弟节点，不占用放大计算的索引） */
  dividerBefore?: boolean
}

interface DockItemProps {
  item: NavItem
  active: boolean
  onClick: () => void
  /** 鼠标悬停回调（用于路由 chunk 预加载） */
  onHover?: (id: string) => void
  /** 父级广播的指针 Y（视口坐标；Infinity = 指针已离开 Dock） */
  mouseY: MotionValue<number>
  spring: SpringOptions
  baseItemSize: number
  /** 光标正下方的目标尺寸（高斯曲线峰值） */
  magnification: number
  /** 高斯衰减宽度：越小，"拉近感"越集中在光标附近 */
  sigma: number
}

export function DockItem({
  item,
  active,
  onClick,
  onHover,
  mouseY,
  spring,
  baseItemSize,
  magnification,
  sigma
}: DockItemProps) {
  const { labelKey, icon } = item
  const { t } = useI18n()
  const label = t(labelKey)
  const wrapperRef = useRef<HTMLDivElement>(null)

  // 自身中心 Y（视口坐标；NaN = 尚未测量）。
  // 与 mouseY 同为 MotionValue，构成高斯计算的两个输入源：
  // 任一变化即触发重算，不捕获会过期的闭包变量，也不依赖重渲染时机
  const centerY = useMotionValue(NaN)

  // 挂载后测量一次（scale 不改变布局，中心恒定）；窗口尺寸变化时重测
  useLayoutEffect(() => {
    const measure = () => {
      const el = wrapperRef.current
      if (!el) return
      const r = el.getBoundingClientRect()
      centerY.set(r.top + r.height / 2)
    }
    measure()
    window.addEventListener('resize', measure)
    return () => window.removeEventListener('resize', measure)
  }, [centerY])

  // 高斯放大曲线：光标附近急速放大、远离快速衰减（macOS "拉近"感）。
  // Number() 包裹是为了兼容 useTransform 数组重载的参数类型
  const size = useTransform([mouseY, centerY], (latest) => {
    const y = Number(latest[0])
    const cy = Number(latest[1])
    if (!Number.isFinite(y) || !Number.isFinite(cy)) return baseItemSize
    const d = (y - cy) / sigma
    return baseItemSize + (magnification - baseItemSize) * Math.exp(-0.5 * d * d)
  })

  // spring 平滑 → GPU 加速的 scale（wrapper 固定占位，放大不影响布局）
  const smoothSize = useSpring(size, spring)
  const scale = useTransform(smoothSize, (s) => s / baseItemSize)

  return (
    <Popover
      content={label}
      placement="right"
      mouseEnterDelay={0}
      mouseLeaveDelay={0}
      align={{ offset: [14, 0] }}
    >
      {/*
        外层 wrapper：固定 baseItemSize，作为 flex 占位 + 键盘焦点锚点，
        不参与任何动画计算，彻底杜绝 layout thrashing
      */}
      <div
        ref={wrapperRef}
        className="dock-item-wrapper"
        data-dock-item
        style={{ width: baseItemSize, height: baseItemSize }}
        role="button"
        aria-label={label}
        aria-current={active ? 'page' : undefined}
        tabIndex={active ? 0 : -1}
        onClick={onClick}
        onMouseEnter={(e) => {
          // 主测量通道：直接从事件目标读取，不依赖 ref 在 Popover 内部的传递
          const r = e.currentTarget.getBoundingClientRect()
          centerY.set(r.top + r.height / 2)
          // 路由预加载：hover 时触发 chunk 预加载，点击时直接命中缓存
          onHover?.(item.id)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            onClick()
          }
        }}
      >
        {/*
          视觉层：绝对定位 + inset:0 撑满 wrapper，
          通过 transform: scale 实现 GPU 加速放大，不影响 flex 布局
        */}
        <motion.div
          className="dock-visual"
          data-active={active || undefined}
          style={{ scale }}
          initial={false}
        >
          {/* 激活白底：置于 visual 内层，跟随父级 scale 一起放大 */}
          <motion.div
            className="dock-active-bg"
            initial={false}
            animate={{ scale: active ? 1 : 0.6, opacity: active ? 1 : 0 }}
            transition={{ type: 'spring', stiffness: 380, damping: 24, mass: 0.6 }}
          />
          <span className="dock-icon">{icon}</span>
        </motion.div>
      </div>
    </Popover>
  )
}
