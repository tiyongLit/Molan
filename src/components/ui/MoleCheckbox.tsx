import type { MouseEventHandler, ReactNode, CSSProperties } from 'react'
import { SEMANTIC_COLORS } from '@/constants/theme'

/**
 * 全应用统一勾选框：蓝色 #3B82F6 选中 + 白色对勾。
 * Clean / Optimize / Analyze 共用同一勾选语言，禁止各页自行复制样式。
 *
 * 三态：
 *  - checked：蓝底 + 白勾
 *  - partial：蓝底 + 灰白 2px 内边（可加中间标记）
 *  - off：淡白底 + 白边
 */
export interface MoleCheckboxProps {
  checked: boolean
  /** 半选态 */
  partial?: boolean
  disabled?: boolean
  /** 半选态中间标记（如 Optimize 的 "–"），默认不显示 */
  partialMark?: ReactNode
  onClick?: MouseEventHandler<HTMLButtonElement>
  className?: string
}

const BASE_CLASS =
  'w-3.5 h-3.5 rounded-[2px] border-[1.5px] flex items-center justify-center shrink-0 transition-all'

function checkboxStyle(checked: boolean, partial: boolean, disabled: boolean): CSSProperties {
  if (disabled) {
    return { borderColor: 'rgba(255,255,255,0.15)', backgroundColor: 'rgba(255,255,255,0.05)' }
  }
  if (checked) {
    return { borderColor: SEMANTIC_COLORS.accentBlue, backgroundColor: SEMANTIC_COLORS.accentBlue }
  }
  if (partial) {
    return { border: '2px solid rgba(255, 255, 255, 0.85)', backgroundColor: SEMANTIC_COLORS.accentBlue }
  }
  return { borderColor: 'rgba(255,255,255,0.25)', backgroundColor: 'rgba(255,255,255,0.1)' }
}

function CheckIcon() {
  return (
    <svg className="w-2 h-2" fill="none" viewBox="0 0 24 24" stroke="#FFFFFF" strokeWidth={3}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  )
}

export function MoleCheckbox({
  checked,
  partial = false,
  disabled = false,
  partialMark,
  onClick,
  className = '',
}: MoleCheckboxProps) {
  return (
    <button
      type="button"
      className={`${BASE_CLASS} ${className}`}
      style={checkboxStyle(checked, partial, disabled)}
      disabled={disabled}
      onClick={onClick}
    >
      {checked && <CheckIcon />}
      {partial && !checked && partialMark}
    </button>
  )
}
