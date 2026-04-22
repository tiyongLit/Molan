import { useState } from 'react'
import { CloseCircleFilled, DownOutlined, WarningOutlined } from '@ant-design/icons'
import { ResultView } from '@/components/ui'
import { PAGE_THEME_VARS, SEMANTIC_COLORS } from '@/constants/theme'
import { useI18n } from '@/i18n'

export interface OptimizeResultSummary {
  applied: number
  failed: number
  skipped: number
}

/** 失败明细条目：id 用于 React key，name 用户可见，reason 为后端回填原因（可能缺失） */
export interface OptimizeFailedItem {
  id: string
  name: string
  reason?: string
}

export interface OptimizeResultProps {
  /** 执行结果摘要 */
  summary: OptimizeResultSummary | null
  /** 失败明细（仅含失败项，与 summary.failed 同源） */
  failedItems?: OptimizeFailedItem[]
  /** 执行错误信息（非空表示执行失败） */
  error: string | null
  /** 重新分析 */
  onRestart: () => void
  /** 完成，返回首页 */
  onFinish: () => void
}

/**
 * 失败明细卡片：默认展开，点击标题行折叠/展开。
 * 仅存在失败项时由父组件渲染——用户可追溯"哪几项失败、为什么"，
 * 成功项不进列表（只列失败的）。
 */
function FailedDetailCard({ items }: { items: OptimizeFailedItem[] }) {
  const { t } = useI18n()
  const [collapsed, setCollapsed] = useState(false)

  return (
    <div className="w-full overflow-hidden rounded-2xl border border-white/[0.12] bg-white/[0.04]">
      {/* 标题行：整行可点，负责折叠/展开 */}
      <button
        type="button"
        onClick={() => setCollapsed((v) => !v)}
        className="flex w-full cursor-pointer select-none items-center gap-2 border-0 bg-transparent px-4 py-3 text-left"
      >
        <WarningOutlined style={{ color: SEMANTIC_COLORS.warningYellow, fontSize: 14 }} />
        <span className="flex-1 text-[13px] font-medium text-white/85">
          {t('optimize.result.detailTitle', { count: items.length })}
        </span>
        <DownOutlined
          className={`text-[11px] text-white/45 transition-transform duration-200 ${collapsed ? '-rotate-90' : ''}`}
        />
      </button>
      {/* grid-template-rows 0fr↔1fr 过渡：平滑折叠且无需测量内容高度 */}
      <div className={`grid transition-[grid-template-rows] duration-300 ease-out ${collapsed ? 'grid-rows-[0fr]' : 'grid-rows-[1fr]'}`}>
        <div className="overflow-hidden">
          <ul className="m-0 flex list-none flex-col gap-2.5 px-4 pt-1 pb-4">
            {items.map((item) => (
              <li key={item.id} className="flex items-start gap-2">
                <CloseCircleFilled style={{ color: SEMANTIC_COLORS.dangerRed, fontSize: 12, marginTop: 4 }} />
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] leading-5 text-white/85">{item.name}</div>
                  <div className="text-[12px] leading-5 text-white/45">
                    {item.reason || t('optimize.result.reasonUnknown')}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  )
}

/**
 * 优化完成结果页：基于通用 ResultView，仅负责优化域的状态/文案计算。
 * 三种状态：
 *  - success：全部成功
 *  - warning：有失败或跳过项
 *  - error：执行失败（error 非空）
 * 失败时在结果区下方追加可折叠的失败明细卡片（成功项不展示）。
 */
export default function OptimizeResult({ summary, failedItems = [], error, onRestart, onFinish }: OptimizeResultProps) {
  const { t } = useI18n()
  const isError = Boolean(error)
  const hasPartial = !isError && (summary ? summary.failed > 0 || summary.skipped > 0 : false)
  const status: 'success' | 'warning' | 'error' = isError ? 'error' : hasPartial ? 'warning' : 'success'

  const title = isError
    ? <span style={{ color: SEMANTIC_COLORS.dangerRed }}>{t('optimize.result.failed')}</span>
    : t('optimize.result.success', { count: summary?.applied ?? 0 })

  // 副标题：失败数用警示红、跳过数降为次要白，二者以分隔符连接
  const subTitle = isError ? (
    error
  ) : hasPartial && summary ? (
    <>
      {summary.failed > 0 && (
        <span style={{ color: SEMANTIC_COLORS.dangerRed }}>
          {t('optimize.result.failedItems', { count: summary.failed })}
        </span>
      )}
      {summary.failed > 0 && summary.skipped > 0 && t('optimize.result.itemSep')}
      {summary.skipped > 0 && (
        <span className="text-white/60">{t('optimize.result.skippedItems', { count: summary.skipped })}</span>
      )}
    </>
  ) : undefined

  const showFailedDetail = !isError && (summary?.failed ?? 0) > 0 && failedItems.length > 0

  return (
    <ResultView
      status={status}
      title={title}
      subtitle={subTitle}
      secondaryLabel={t('optimize.result.restart')}
      onSecondary={onRestart}
      primaryLabel={t('optimize.result.finish')}
      onPrimary={onFinish}
      style={PAGE_THEME_VARS}
    >
      {showFailedDetail && <FailedDetailCard items={failedItems} />}
    </ResultView>
  )
}
