import type { ReactNode } from 'react'

export interface BadgeProps {
  /** 徽标文案 */
  label: string
  /** 前景色 */
  color: string
  /** 背景色 */
  bg: string
  /** 可选前置图标 */
  icon?: ReactNode
  /** 可选 hover 提示 */
  title?: string
}

/**
 * 小型圆角徽标（来源 / 类型 / 错误标记等）。
 *
 * 统一「更新来源 sourceChip / 启动项类型 kindChip / 损坏 Error 徽标」
 * 三处 chip 的结构重复，样式一处维护。
 */
export function Badge({ label, color, bg, icon, title }: BadgeProps) {
  return (
    <span
      title={title}
      className="text-[9px] px-1.5 py-0.5 rounded-full font-medium shrink-0 inline-flex items-center gap-1"
      style={{ color, background: bg }}
    >
      {icon}
      {label}
    </span>
  )
}
