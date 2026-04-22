import { useState, useEffect } from 'react'
import { CheckCircle, XCircle } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import useTauri from '@/hooks/useTauri'
import { moleMessage } from '@/components/ui'
import { formatSize } from '@/utils/format'
import { useI18n } from '@/i18n'
import type { UninstallHistoryRecord } from '@/types/uninstall-history'

/**
 * 卸载历史列表组件：以列表形式展示卸载历史记录
 * 
 * 与卸载列表保持一致的布局和样式，支持：
 * - 列表展示历史记录
 * - 清空历史功能
 */
export function UninstallHistoryList() {
  const tauri = useTauri()
  const { t, locale } = useI18n()
  const [records, setRecords] = useState<UninstallHistoryRecord[]>([])
  const [loading, setLoading] = useState(false)

  // 加载历史记录
  const loadHistory = async () => {
    setLoading(true)
    try {
      const result = await tauri.mole_get_uninstall_history() as { records: UninstallHistoryRecord[] }
      setRecords(result.records || [])
    } catch (err) {
      console.error('加载历史记录失败', err)
      moleMessage.error(t('uninstall.history.loadFailed'))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadHistory()
  }, [])

  // 清空历史
  const handleClearHistory = async () => {
    try {
      await tauri.mole_clear_uninstall_history()
      setRecords([])
    } catch (err) {
      console.error('清空历史记录失败', err)
      moleMessage.error(t('uninstall.history.clearFailed'))
    }
  }

  // 格式化时间
  const formatTime = (timestamp: string) => {
    const date = new Date(timestamp)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffMins = Math.floor(diffMs / 60000)
    const diffHours = Math.floor(diffMs / 3600000)
    const diffDays = Math.floor(diffMs / 86400000)

    if (diffMins < 1) return t('uninstall.time.justNow')
    if (diffMins < 60) return t('uninstall.time.minutesAgo', { count: diffMins })
    if (diffHours < 24) return t('uninstall.time.hoursAgo', { count: diffHours })
    if (diffDays < 7) return t('uninstall.time.daysAgo', { count: diffDays })
    return date.toLocaleDateString(locale)
  }

  return (
    <div className="h-full flex flex-col">
      {/* 顶部操作栏 */}
      <div className="shrink-0 flex items-center justify-between px-6 py-3 border-b border-white/[0.14]">
        <div className="flex items-center gap-2">
          <span className="text-[13px] text-white/85">
            {t('uninstall.history.total', { count: records.length })}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={handleClearHistory}
            disabled={records.length === 0}
            className="text-[11px] text-white/60 hover:text-white transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
          >
            {t('uninstall.history.clear')}
          </button>
        </div>
      </div>

      {/* 历史列表 */}
      <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: 'calc(100% - 60px)' }}>
        <div className="py-2">
          {loading ? (
            <div className="flex items-center justify-center h-32">
              <div className="text-sm text-white/60">{t('uninstall.history.loading')}</div>
            </div>
          ) : records.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-64 text-white/60">
              <p className="text-xs text-white/40 mt-2">{t('uninstall.history.empty')}</p>
            </div>
          ) : (
            <div className="space-y-0.5">
              {records.map((record, index) => {
                // 计算是否成功：如果所有文件都被删除（废纸篓或永久删除），则视为成功
                const totalDeleted = record.trashed_count + record.sudo_removed_count
                const isSuccess = totalDeleted === record.file_count
                const operationType = record.data_only ? 'clearData' : 'uninstall'
                const operationText = operationType === 'uninstall' ? t('uninstall.history.fullUninstall') : t('uninstall.history.dataOnly')

                return (
                  <div key={record.id || index} className="mr-[24px]">
                    <div className="flex items-center gap-2.5 px-[24px] py-2 rounded-lg transition-colors hover:bg-black/[0.25]">
                      {/* 状态图标 */}
                      <div className="shrink-0">
                        {isSuccess ? (
                          <CheckCircle size={16} className="text-emerald-400" />
                        ) : (
                          <XCircle size={16} className="text-red-400" />
                        )}
                      </div>

                      {/* 应用信息 */}
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
                            {record.app_name}
                          </span>
                          <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/[0.08] text-white/60">
                            {operationText}
                          </span>
                        </div>
                        <div className="text-[10px] font-mono text-white/60 truncate">
                          {t('uninstall.history.meta', {
                            time: formatTime(record.timestamp),
                            count: record.file_count,
                            size: formatSize(record.total_size_bytes),
                          })}
                        </div>
                        <div className="flex items-center gap-2 mt-0.5 text-[10px]">
                          {record.trashed_count > 0 && (
                            <span className="text-emerald-400/80">
                              {t('uninstall.history.trashed', { count: record.trashed_count })}
                            </span>
                          )}
                          {record.sudo_removed_count > 0 && (
                            <span className="text-red-400/80">
                              {t('uninstall.history.removed', { count: record.sudo_removed_count })}
                            </span>
                          )}
                        </div>
                      </div>

                      {/* 右侧：状态 */}
                      <span className="text-[11px] shrink-0">
                        {isSuccess ? (
                          <span className="text-emerald-400">{t('uninstall.history.success')}</span>
                        ) : (
                          <span className="text-red-400">{t('uninstall.history.failed')}</span>
                        )}
                      </span>
                    </div>
                  </div>
                )
              })}
            </div>
          )}
        </div>
      </SimpleBar>
    </div>
  )
}
