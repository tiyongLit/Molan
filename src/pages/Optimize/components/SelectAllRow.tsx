import { MoleCheckbox } from '@/components/ui'
import { SEMANTIC_COLORS } from '@/constants/theme'

export interface SelectAllRowProps {
  /** 是否全选 */
  allChecked: boolean
  /** 是否部分选中（半选态） */
  partialSelected: boolean
  /** 是否禁用（executing 阶段） */
  disabled: boolean
  /** 任务总数 */
  taskCount: number
  /** 已选数量 */
  selectedCount: number
  /** 切换全选（传入当前是否全选，由调用方决定目标状态） */
  onToggleAll: (allChecked: boolean) => void
}

/**
 * 全选行（preview / executing 阶段列表顶部）。
 * 整行可点击切换全选/取消全选。
 */
export default function SelectAllRow({
  allChecked,
  partialSelected,
  disabled,
  taskCount,
  selectedCount,
  onToggleAll,
}: SelectAllRowProps) {
  return (
    <div
      className={`h-10 my-1 flex items-center px-[24px] gap-3 ${disabled ? '' : 'cursor-pointer hover:bg-white/[0.06]'} transition-colors`}
      onClick={() => !disabled && onToggleAll(allChecked)}
    >
      <span className="w-4 h-4 flex items-center justify-center text-white/40 text-sm">☰</span>
      <MoleCheckbox
        checked={allChecked}
        partial={partialSelected}
        disabled={disabled}
        partialMark={<span className="text-white text-[9px] leading-none">–</span>}
      />
      <span className="text-sm font-medium text-white shrink-0">全选</span>
      <span className="text-xs text-white/50">
        共 {taskCount} 项，已选
        <span className="ml-1 font-medium" style={{ color: SEMANTIC_COLORS.warningYellow }}>{selectedCount}</span>
      </span>
    </div>
  )
}
