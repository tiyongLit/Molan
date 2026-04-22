import { memo } from 'react'
import { getColorLevel, BAR_COLOR_CLASS } from '../utils/sizeColor'

/**
 * 共享进度条 — EntryRow 和 LocationSelector 共用
 */
export const ProgressBar = memo(function ProgressBar({ percent }: { percent: number }) {
  return (
    <div className="h-1 rounded-full bg-black/[0.3] overflow-hidden mt-1.5 w-full">
      <div
        className={`h-full rounded-full transition-all duration-500 ease-out ${BAR_COLOR_CLASS[getColorLevel(percent)]}`}
        style={{ width: `${Math.max(percent, 1.5)}%` }}
      />
    </div>
  )
})
