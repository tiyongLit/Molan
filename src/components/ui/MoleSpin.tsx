import { useMemo } from 'react'

interface MoleSpinProps {
  size?: 'small' | 'default' | 'large'
  tip?: string
  description?: string
  className?: string
  style?: React.CSSProperties
  children?: React.ReactNode
}

const sizeMap = {
  small: { ring: 20, stroke: 2 },
  default: { ring: 36, stroke: 3 },
  large: { ring: 48, stroke: 3.5 }
}

const keyframesStyle = `
@keyframes mole-spin-rotate {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}
@keyframes mole-spin-glow {
  0%, 100% { opacity: 0.6; }
  50% { opacity: 1; }
}
`

let injected = false
export function ensureKeyframes() {
  if (injected || typeof document === 'undefined') return
  const style = document.createElement('style')
  style.textContent = keyframesStyle
  document.head.appendChild(style)
  injected = true
}

export function MoleSpin({
  size = 'default',
  tip,
  description,
  className,
  style,
  children
}: MoleSpinProps) {
  ensureKeyframes()

  const { ring, stroke } = sizeMap[size]
  const half = ring / 2
  const r = half - stroke
  const circumference = 2 * Math.PI * r
  const dash = circumference * 0.7

  const gradientId = useMemo(() => `mole-spin-grad-${Math.random().toString(36).slice(2, 8)}`, [])

  const ringEl = (
    <svg
      width={ring}
      height={ring}
      viewBox={`0 0 ${ring} ${ring}`}
      style={{
        animation: 'mole-spin-rotate 1s cubic-bezier(0.4, 0, 0.2, 1) infinite',
        filter: 'drop-shadow(0 0 6px rgba(168, 85, 247, 0.4))'
      }}
    >
      <defs>
        <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#fbbf24" />
          <stop offset="50%" stopColor="#fb923c" />
          <stop offset="100%" stopColor="#f97316" />
        </linearGradient>
      </defs>
      <circle
        cx={half}
        cy={half}
        r={r}
        fill="none"
        stroke={`url(#${gradientId})`}
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={`${dash} ${circumference - dash}`}
      />
    </svg>
  )

  const text = tip || description
  const inner = (
    <div
      className={className}
      style={{
        display: 'inline-flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: text ? 12 : 0,
        ...style
      }}
    >
      {ringEl}
      {text && (
        <span
          style={{
            color: 'var(--text-secondary)',
            fontSize: size === 'small' ? 11 : size === 'large' ? 14 : 13,
            animation: 'mole-spin-glow 2s ease-in-out infinite',
            whiteSpace: 'nowrap'
          }}
        >
          {text}
        </span>
      )}
    </div>
  )

  if (children) {
    return (
      <div style={{ position: 'relative', display: 'inline-block' }}>
        <div style={{ opacity: 0.4, pointerEvents: 'none' }}>{children}</div>
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center'
          }}
        >
          {inner}
        </div>
      </div>
    )
  }

  return inner
}

export type { MoleSpinProps }
