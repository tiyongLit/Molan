import { MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'
import { useI18n } from '@/i18n'
import type { MoleOptimizeTask } from '@/types/mole'
import { groupIconMap, type OptimizeGroupDef, type TaskRuntime } from '../optimize.constants'
import OptimizeTaskRow from './OptimizeTaskRow'

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`w-4 h-4 text-white/40 transition-transform ${expanded ? 'rotate-90' : ''}`}
      fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
    </svg>
  )
}

export interface OptimizeGroupRowProps {
  group: OptimizeGroupDef
  /** 该组内的任务（已由调用方按 actionIds 过滤） */
  tasks: MoleOptimizeTask[]
  isExpanded: boolean
  isIdle: boolean
  isExecuting: boolean
  selectedActions: Set<string>
  taskRuntime: Map<string, TaskRuntime>
  onToggleGroup: (group: OptimizeGroupDef) => void
  onToggleExpand: (groupId: string) => void
  onToggleAction: (actionId: string) => void
}

/**
 * 分组行 + 展开的任务列表（对齐 Clean CategoryRow 模式）。
 * 任务行由 OptimizeTaskRow（memo）渲染，执行阶段仅受影响行重渲染。
 */
export default function OptimizeGroupRow({
  group,
  tasks,
  isExpanded,
  isIdle,
  isExecuting,
  selectedActions,
  taskRuntime,
  onToggleGroup,
  onToggleExpand,
  onToggleAction,
}: OptimizeGroupRowProps) {
  const { t } = useI18n()
  const icon = groupIconMap[group.id]
  const disabled = isExecuting || isIdle
  const groupAllSelected = tasks.every((t) => selectedActions.has(t.id))
  const groupPartial = !groupAllSelected && tasks.some((t) => selectedActions.has(t.id))
  const selectedInGroup = tasks.filter((t) => selectedActions.has(t.id)).length

  return (
    <div>
      {/* ── 分组标题行 ── */}
      <div
        className={`h-10 my-1 flex items-center px-[24px] gap-3 ${disabled ? '' : 'cursor-pointer hover:bg-white/[0.06]'} transition-colors`}
        onClick={() => !disabled && onToggleExpand(group.id)}
      >
        {icon}
        <MoleCheckbox
          checked={groupAllSelected}
          partial={groupPartial}
          disabled={disabled}
          partialMark={<span className="text-white text-[9px] leading-none">–</span>}
          onClick={(e) => {
            e.stopPropagation()
            if (!disabled) onToggleGroup(group)
          }}
        />
        <span className="text-sm font-medium text-white shrink-0">{t(group.titleKey)}</span>
        <span className="text-xs text-white/50">
          {t('optimize.group.selectedPre')}
          <span className="ml-1 font-medium" style={{ color: SEMANTIC_COLORS.warningYellow }}>
            {selectedInGroup}
          </span>
          {t('optimize.group.selectedPost', { total: tasks.length })}
        </span>
        <div className="flex-1" />
        {!disabled && <ChevronIcon expanded={isExpanded} />}
      </div>

      {/* ── 展开的任务行（CSS grid 过渡） ── */}
      {(isIdle || isExpanded) && (
        <div
          className={`grid overflow-hidden transition-[grid-template-rows,opacity] duration-250 ease-[cubic-bezier(0.4,0,0.2,1)] ${isIdle || isExpanded ? 'opacity-100' : 'opacity-0'}`}
          style={{ gridTemplateRows: isIdle || isExpanded ? '1fr' : '0fr' }}
        >
          <div className="min-h-0">
            {tasks.map((task) => (
              <OptimizeTaskRow
                key={task.id}
                task={task}
                isSelected={selectedActions.has(task.id)}
                disabled={disabled}
                isExecuting={isExecuting}
                runtime={taskRuntime.get(task.id)}
                onToggle={onToggleAction}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
