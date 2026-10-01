import classnames from 'classnames'
import { MoleButton, ScanEllipsis, ScanErrorPlaceholder } from '@/components/ui'
import { useI18n } from '@/i18n'
import type { MoleOptimizeSystemInfo } from '@/types/mole'

/**
 * 优化页左上角状态条（对齐 CleanStatusBar 模式）
 *
 * 状态：
 * - idle      初始态（标题 + 副标题 + 错误提示占位）
 * - analyzing  正在分析系统（循环省略号 + 副行说明）
 * - preview    分析完成（共发现 N 项可优化 + 系统信息 + 返回按钮）
 * - executing  正在执行优化（循环省略号 + 当前任务名）
 */
export type OptimizeStatus = 'idle' | 'analyzing' | 'preview' | 'executing'

export interface OptimizeStatusBarProps {
  status: OptimizeStatus
  /** idle 状态：分析失败的错误信息（固定占位，避免列表抖动） */
  error?: string
  /** preview 状态：可优化任务总数 */
  taskCount?: number
  /** preview 状态：已勾选数量 */
  selectedCount?: number
  /** preview 状态：系统信息（RAM / Disk / uptime） */
  systemInfo?: MoleOptimizeSystemInfo
  /** executing 状态：当前正在执行的任务名 */
  currentTask?: string
  /** preview 状态：返回按钮回调 */
  onBack?: () => void
  className?: string
}

export default function OptimizeStatusBar({
  status,
  error,
  taskCount = 0,
  selectedCount = 0,
  systemInfo,
  currentTask,
  onBack,
  className,
}: OptimizeStatusBarProps) {
  const { t } = useI18n()

  if (status === 'idle') {
    return (
      <div className={classnames('flex flex-col pt-1 min-h-[80px]', className)}>
        <h1 className="text-2xl font-semibold leading-tight text-white">{t('optimize.status.idle.title')}</h1>
        <p className="mt-2 text-sm text-white/60">{t('optimize.status.idle.subtitle')}</p>
        {/* 错误提示固定占位（min-h-[16px]），避免出现/消失时列表下移 */}
        <ScanErrorPlaceholder error={error} />
      </div>
    )
  }

  if (status === 'analyzing') {
    return (
      <div className={classnames('pt-1', className)}>
        <h1 className="text-2xl font-semibold leading-tight text-white">
          {t('optimize.status.analyzing.title')}<ScanEllipsis />
        </h1>
        <div className="mt-2 flex items-center gap-2">
          <span className="text-sm text-white/60">{t('optimize.status.analyzing.subtitle')}</span>
        </div>
      </div>
    )
  }

  if (status === 'executing') {
    return (
      <div className={classnames('pt-1', className)}>
        <h1 className="text-2xl font-semibold leading-tight text-white">
          {t('optimize.status.executing.title')}<ScanEllipsis />
        </h1>
        <div className="mt-2 flex items-center gap-2">
          <span className="text-sm text-white/60">
            {currentTask ? t('optimize.status.executing.current', { task: currentTask }) : t('optimize.status.executing.noTask')}
          </span>
        </div>
      </div>
    )
  }

  // preview
  return (
    <div className={classnames('pt-1', className)}>
      <div className="flex items-center gap-3">
        <h1 className="text-2xl font-semibold leading-tight text-white">
          {t('optimize.status.preview.title', { count: taskCount })}
        </h1>
        {onBack && (
          <MoleButton size="small" variant="outlined" className="optimize-back-btn" onClick={onBack}>
            {t('optimize.status.preview.back')}
          </MoleButton>
        )}
      </div>
      <div className="mt-2 flex items-center gap-1">
        <span className="text-sm text-white/60">{t('optimize.status.preview.selectedPre')}</span>
        <span className="text-sm font-semibold text-[#fb923c]">{selectedCount}</span>
        <span className="text-sm text-white/60">{t('optimize.status.preview.selectedPost')}</span>
        {systemInfo && (
          <>
            <span className="text-sm text-white/60 mx-1">·</span>
            <span className="text-sm text-white/60">
              {systemInfo.memory_used_gb.toFixed(1)}/{systemInfo.memory_total_gb.toFixed(0)} GB RAM
            </span>
            <span className="text-sm text-white/60 mx-1">·</span>
            <span className="text-sm text-white/60">
              {systemInfo.disk_used_gb.toFixed(0)}/{systemInfo.disk_total_gb.toFixed(0)} GB Disk
            </span>
          </>
        )}
      </div>
    </div>
  )
}
