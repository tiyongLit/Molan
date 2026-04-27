import * as React from 'react'
import { clsx } from 'clsx'
import styles from './ProgressDial.module.scss'

import { validProgress } from './utils'

export interface ProgressDialProps {
  /** 进度值 0-100 */
  percent?: number
  /** 组件整体尺寸，默认 160 */
  size?: number
  /** 进度环颜色，默认 #4B8DF8 */
  strokeColor?: string
  /** 轨道颜色，默认半透白（适配深色背景） */
  railColor?: string
  /** 环境辉光色（Liquid Glass 风格），默认不渲染 */
  glowColor?: string
  /** 环粗细，默认 8 */
  strokeWidth?: number
  /** 自定义内容 */
  children?: React.ReactNode
  /** 三角底边一半宽度，默认 10 */
  pointerWidth?: number
  /** 三角向外突出长度，默认 10 */
  pointerLength?: number
  /** 自定义 class */
  className?: string
  /** 自定义样式 */
  style?: React.CSSProperties
}

const ProgressDial: React.FC<ProgressDialProps> = ({
  percent = 0,
  size = 160,
  strokeColor = '#4B8DF8',
  railColor = 'rgba(255,255,255,0.10)',
  glowColor,
  strokeWidth = 8,
  children,
  pointerWidth = 8,
  pointerLength = 8,
  className,
  style
}) => {
  // ======================== 百分比处理 ========================
  const validPct = validProgress(percent)

  // ======================== SVG 几何计算 ========================
  // 使用 viewBox="0 0 100 100" 坐标系，中心在 (50, 50)
  const center = 50
  // 环的半径（中线），保证外边缘不超出 viewBox
  const ringRadius = 50 - strokeWidth / 2
  const circumference = 2 * Math.PI * ringRadius

  // 进度条偏移量：从 12 点钟方向起始（通过 rotate(-90) 实现）
  const strokeLength = (validPct / 100) * circumference
  const strokeDashoffset = circumference - strokeLength

  // 三角指针旋转角度
  const rotation = (validPct / 100) * 360

  // 三角指针坐标（12 点钟方向，再通过 rotate 旋转到对应位置）
  const baseY = strokeWidth // 底边位于环内边缘
  const tipY = -pointerLength // 尖端向外突出
  const pointerPoints = `
    ${center},${tipY}
    ${center - pointerWidth},${baseY}
    ${center + pointerWidth},${baseY}
  `

  // 内圆半径：环内边缘以内
  const innerRadius = ringRadius - strokeWidth / 2

  return (
    <div
      className={clsx(styles.progressWrapper, className)}
      style={{ width: size, height: size, ...style }}
    >
      <svg width="100%" height="100%" viewBox="0 0 100 100" className={styles.svg}>
        {/* 1. 环境辉光环（Liquid Glass 氛围层） */}
        {glowColor && (
          <circle
            cx={center}
            cy={center}
            r={ringRadius}
            fill="none"
            stroke={glowColor}
            strokeWidth={strokeWidth * 2.5}
            strokeLinecap="round"
            opacity={0.12}
            className={styles.glowRing}
          />
        )}

        {/* 2. 轨道（底环） */}
        <circle
          cx={center}
          cy={center}
          r={ringRadius}
          fill="none"
          stroke={railColor}
          strokeWidth={strokeWidth}
          strokeLinecap="butt"
        />

        {/* 3. 进度弧（带 CSS 平滑动效） */}
        <circle
          cx={center}
          cy={center}
          r={ringRadius}
          fill="none"
          stroke={strokeColor}
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={strokeDashoffset}
          strokeLinecap="butt"
          transform={`rotate(-90 ${center} ${center})`}
          className={styles.progressArc}
        />

        {/* 4. 三角指针（0% 和 100% 时不显示） */}
        {validPct > 0 && validPct < 100 && (
          <polygon
            points={pointerPoints}
            fill={strokeColor}
            className={styles.pointer}
            transform={`rotate(${rotation} ${center} ${center})`}
          />
        )}

        {/* 5. 内圆：液态玻璃覆盖层 */}
        <circle
          cx={center}
          cy={center}
          r={innerRadius}
          className={styles.innerGlass}
        />
      </svg>

      {/* 6. 自定义内容区域 */}
      {children && <div className={styles.contentWrapper}>{children}</div>}
    </div>
  )
}

export default ProgressDial
