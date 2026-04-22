import { ResultView } from '@/components/ui'
import { formatSize } from '@/utils/format'
import { PAGE_THEME_VARS } from '@/constants/theme'
import { useI18n } from '@/i18n'

export interface ScanResultProps {
  /** 清理结果摘要 */
  doneSummary: { totalCleaned: number; failedCount: number; permanentDelete: boolean } | null
  /** 清理错误信息（非空表示清理失败） */
  scanError: string | null
  /** 删除模式：true=直接删除，false=移到废纸篓 */
  permanentDelete: boolean
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
export default function ScanResult({ doneSummary, scanError, permanentDelete, onRescan, onFinish }: ScanResultProps) {
  const { t } = useI18n()
  const isError = Boolean(scanError)
  const hasPartialFailure = !isError && (doneSummary?.failedCount ?? 0) > 0
  const status: 'success' | 'warning' | 'error' = isError
    ? 'error'
    : hasPartialFailure
      ? 'warning'
      : 'success'

  const title = isError
    ? t('clean.result.failed')
    : t('clean.result.success', { size: formatSize(doneSummary?.totalCleaned || 0) })

  const subTitle = isError
    ? scanError
    : hasPartialFailure
      ? t('clean.result.partialFailed', { count: doneSummary?.failedCount ?? 0 })
      : !permanentDelete
        ? t('clean.result.trashHint')
        : undefined

  return (
    <ResultView
      status={status}
      title={title}
      subtitle={subTitle}
      secondaryLabel={t('clean.result.rescan')}
      onSecondary={onRescan}
      primaryLabel={t('clean.result.finish')}
      onPrimary={onFinish}
      style={PAGE_THEME_VARS}
    />
  )
}
