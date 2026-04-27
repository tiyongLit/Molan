import { ResultView } from '@/components/ui'
import { formatSize } from '@/utils/format'
import { PAGE_THEME_VARS } from '@/constants/theme'

export interface ScanResultProps {
  /** 清理结果摘要 */
  doneSummary: { totalCleaned: number; failedCount: number } | null
  /** 清理错误信息（非空表示清理失败） */
  scanError: string | null
  /** 重新扫描 */
  onRescan: () => void
  /** 完成，返回首页 */
  onFinish: () => void
}

/**
 * 扫描/清理完成结果页：基于通用 ResultView，仅负责清理域的状态/文案计算。
 * 三种状态：
 *  - success：清理完成，无失败项
 *  - warning：清理完成，但有部分项失败或跳过
 *  - error：清理失败（scanError 非空）
 */
export default function ScanResult({ doneSummary, scanError, onRescan, onFinish }: ScanResultProps) {
  const isError = Boolean(scanError)
  const hasPartialFailure = !isError && (doneSummary?.failedCount ?? 0) > 0
  const status: 'success' | 'warning' | 'error' = isError
    ? 'error'
    : hasPartialFailure
      ? 'warning'
      : 'success'

  const title = isError
    ? '清理失败'
    : `清理完成, 共释放 ${formatSize(doneSummary?.totalCleaned || 0)} 磁盘空间`

  const subTitle = isError
    ? scanError
    : hasPartialFailure
      ? `${doneSummary?.failedCount} 项清理失败或跳过`
      : undefined

  return (
    <ResultView
      status={status}
      title={title}
      subtitle={subTitle}
      secondaryLabel="重新扫描"
      onSecondary={onRescan}
      primaryLabel="完成"
      onPrimary={onFinish}
      style={PAGE_THEME_VARS}
    />
  )
}