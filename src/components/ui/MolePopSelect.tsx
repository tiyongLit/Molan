import { useEffect, useRef, useState } from 'react'
import './MolePopSelect.scss'

export interface MolePopSelectOption<T extends string | number = string | number> {
  value: T
  label: string
}

export interface MolePopSelectProps<T extends string | number = string | number> {
  value: T
  options: MolePopSelectOption<T>[]
  onChange: (value: T) => void
  disabled?: boolean
  minWidth?: number
}

/**
 * macOS NSPopUpButton 风格下拉（高度复刻柠檬偏好设置的下拉控件）。
 *
 * 触发器：凹陷圆角盒 + 居中标签 + 右侧蓝色圆角方块内嵌白色上下双箭头；
 * 弹出层：毛玻璃深色菜单，hover 整行强调蓝高亮、选中项前置对勾（原生 macOS 菜单语义）。
 *
 * 为何不用 antd Select：其 DOM 为「左对齐 selection-item + 单个 chevron」，且深色主题
 * 自带近黑底与蓝色 focus 环，要像素级复刻 NSPopUpButton 需与内部结构 / CSS-in-JS 优先级
 * 反复对抗。原生自绘可完全掌控每个像素与交互状态。
 */
export function MolePopSelect<T extends string | number = string | number>({
  value,
  options,
  onChange,
  disabled = false,
  minWidth,
}: MolePopSelectProps<T>) {
  const [open, setOpen] = useState(false)
  const wrapRef = useRef<HTMLDivElement>(null)

  // 点击外部 / Escape 关闭
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const current = options.find((o) => o.value === value)

  return (
    <div
      ref={wrapRef}
      className={`mole-pop${open ? ' is-open' : ''}${disabled ? ' is-disabled' : ''}`}
      style={minWidth ? { minWidth } : undefined}
    >
      <button
        type="button"
        className="mole-pop-trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="mole-pop-label">{current?.label ?? ''}</span>
        <span className="mole-pop-arrow" aria-hidden="true">
          {/* 白色上下双箭头（NSPopUpButton 标志） */}
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path d="M2.5 4 L5 1.8 L7.5 4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M2.5 6 L5 8.2 L7.5 6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </span>
      </button>

      {open && (
        <div className="mole-pop-menu" role="listbox">
          {options.map((o) => {
            const selected = o.value === value
            return (
              <button
                key={String(o.value)}
                type="button"
                role="option"
                aria-selected={selected}
                className={`mole-pop-item${selected ? ' is-selected' : ''}`}
                onClick={() => {
                  onChange(o.value)
                  setOpen(false)
                }}
              >
                <span className="mole-pop-check">{selected ? '✓' : ''}</span>
                <span className="mole-pop-item-label">{o.label}</span>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}
