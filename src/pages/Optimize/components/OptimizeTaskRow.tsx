import { memo } from 'react'
import { MoleCheckbox, MoleSpin } from '@/components/ui'
import { useI18n } from '@/i18n'
import { SEMANTIC_COLORS } from '@/constants/theme'
import type { MoleOptimizeTask } from '@/types/mole'
import type { TaskRuntime } from '../optimize.constants'

export interface OptimizeTaskRowProps {
  task: MoleOptimizeTask
  isSelected: boolean
  /** 是否禁用交互（idle / executing 阶段） */
  disabled: boolean
  /** 是否执行阶段（决定右侧显示运行态标记还是「安全」标记） */
  isExecuting: boolean
  /** 执行阶段运行时状态（含 pending/running/success/skipped/failed） */
  runtime?: TaskRuntime
  onToggle: (actionId: string) => void
}

/** 执行阶段任务右侧状态标记（五态） */
function TaskStateMark({ runtime }: { runtime?: TaskRuntime }) {
  const { t } = useI18n()
  const status = runtime?.status ?? 'pending'
  switch (status) {
    case 'running':
      return <MoleSpin size="small" />
    case 'success':
      return <span className="text-xs font-medium shrink-0" style={{ color: SEMANTIC_COLORS.successGreen }}>✓</span>
    case 'skipped':
      return (
        <span className="text-xs font-medium shrink-0" style={{ color: 'rgba(255,255,255,0.4)' }} title={runtime?.note}>
          ⊝ {runtime?.note || t('optimize.task.skipped')}
        </span>
      )
    case 'failed':
      return <span className="text-xs font-medium shrink-0" style={{ color: SEMANTIC_COLORS.dangerRed }}>✗</span>
    default:
      return <span className="w-1.5 h-1.5 rounded-full bg-white/25 shrink-0" />
  }
}

/**
 * 优化任务行（memo 化）：勾选 + 名称 + 描述 + 右侧状态。
 * 执行阶段右侧显示运行态五态标记；否则显示「安全」圆点。
 */
const OptimizeTaskRow = memo(function OptimizeTaskRow({
  task,
  isSelected,
  disabled,
  isExecuting,
  runtime,
  onToggle,
}: OptimizeTaskRowProps) {
  const { t } = useI18n()
  return (
    <div
      className={`h-8 flex items-center pl-[71px] pr-[24px] gap-2 ${disabled ? '' : 'cursor-pointer hover:bg-white/[0.06]'} transition-colors`}
      onClick={() => !disabled && onToggle(task.id)}
    >
      <MoleCheckbox
        checked={isSelected}
        disabled={disabled}
        onClick={(e) => {
          e.stopPropagation()
          if (!disabled) onToggle(task.id)
        }}
      />
      <span className="text-xs text-white w-[170px] shrink-0 truncate leading-4">{task.name}</span>
      <span className="text-[10px] text-white/40 flex-1 min-w-0 truncate">{task.description}</span>

      {isExecuting ? (
        <TaskStateMark runtime={runtime} />
      ) : (
        <span className="text-[10px] font-medium shrink-0 flex items-center gap-1">
          <span className="w-1.5 h-1.5 rounded-full" style={{ backgroundColor: SEMANTIC_COLORS.successGreen }} />
          <span className="text-white/50">{t('optimize.task.safe')}</span>
        </span>
      )}
    </div>
  )
})

export default OptimizeTaskRow
