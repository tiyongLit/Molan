import { ResultView } from '@/components/ui'
import { PAGE_THEME_VARS } from '@/constants/theme'

export interface OptimizeResultSummary {
  applied: number
  failed: number
  skipped: number
}

export interface OptimizeResultProps {
  /** 执行结果摘要 */
  summary: OptimizeResultSummary | null
  /** 执行错误信息（非空表示执行失败） */
  error: string | null
  /** 重新分析 */
  onRestart: () => void
  /** 完成，返回首页 */
  onFinish: () => void
}

/**
 * 优化完成结果页：基于通用 ResultView，仅负责优化域的状态/文案计算。
 * 三种状态：
 *  - success：全部成功
 *  - warning：有失败或跳过项
 *  - error：执行失败（error 非空）
 */
export default function OptimizeResult({ summary, error, onRestart, onFinish }: OptimizeResultProps) {
  const isError = Boolean(error)
  const hasPartial = !isError && (summary ? summary.failed > 0 || summary.skipped > 0 : false)
  const status: 'success' | 'warning' | 'error' = isError ? 'error' : hasPartial ? 'warning' : 'success'

  const title = isError ? '优化失败' : `优化完成，成功执行 ${summary?.applied ?? 0} 项`

  const subTitle = isError
    ? error
    : hasPartial && summary
      ? [
          summary.failed > 0 ? `${summary.failed} 项失败` : '',
          summary.skipped > 0 ? `${summary.skipped} 项跳过` : '',
        ]
          .filter(Boolean)
          .join('，')
      : undefined

  return (
    <ResultView
      status={status}
      title={title}
      subtitle={subTitle}
      secondaryLabel="重新优化"
      onSecondary={onRestart}
      primaryLabel="完成"
      onPrimary={onFinish}
      style={PAGE_THEME_VARS}
    />
  )
}
