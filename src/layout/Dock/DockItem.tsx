import { useRef, type ReactNode } from 'react'
import { motion, useSpring, useTransform, type MotionValue, type SpringOptions } from 'motion/react'
import { Popover } from 'antd'

interface NavItem {
  id: string
  label: string
  icon: ReactNode
  accent?: string
}

interface DockItemProps {
  item: NavItem
  active: boolean
  onClick: () => void
  dockId: string
  /** 父组件集中管理的目标尺寸 MotionValue（由 FloatingDock 高斯计算后推送） */
  sizeMotion: MotionValue<number>
  spring: SpringOptions
  baseItemSize: number
}

export function DockItem({
  item,
  active,
  onClick,
  dockId,
  sizeMotion,
  spring,
  baseItemSize
}: DockItemProps) {
  const { label, icon } = item
  const ref = useRef<HTMLDivElement>(null)

  // spring 平滑插值 → GPU 加速的 scale 值
  const smoothSize = useSpring(sizeMotion, spring)
  const scale = useTransform(smoothSize, (s) => s / baseItemSize)

  return (
    <Popover
      content={label}
      placement="right"
      mouseEnterDelay={0}
      mouseLeaveDelay={0}
      align={{ offset: [12, 0] }}
    >
      {/*
        外层 wrapper：固定 baseItemSize，作为 flex 布局占位 + Popover 锚点
        不参与任何动画计算，彻底杜绝 layout thrashing
      */}
      <div
        ref={ref}
        className="dock-item-wrapper"
        style={{ width: baseItemSize, height: baseItemSize }}
      >
        {/* 激活态指示器：独立 motion.div，spring 驱动的 scale + opacity */}
        <motion.div
          className="dock-active-bg"
          initial={false}
          animate={{ scale: active ? 1 : 0.5, opacity: active ? 1 : 0 }}
          transition={{ type: 'spring', stiffness: 380, damping: 24, mass: 0.6 }}
        />

        {/*
          视觉层：绝对定位 + inset:0 撑满 wrapper，
          通过 transform: scale 实现 GPU 加速放大，不影响 flex 布局
        */}
        <motion.div
          className="dock-visual"
          data-dock-id={dockId}
          data-active={active || undefined}
          onClick={onClick}
          style={{ scale }}
          initial={false}
        >
          <span className="dock-icon">{icon}</span>
        </motion.div>
      </div>
    </Popover>
  )
}
