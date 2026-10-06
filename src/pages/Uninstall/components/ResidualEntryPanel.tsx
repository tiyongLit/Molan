import { useCallback, useMemo, useRef, useState } from 'react'
import { Archive, ChevronDown, Loader2, Trash2 } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import useTauri from '@/hooks/useTauri'
import { moleMessage, MoleCheckbox } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { useI18n } from '@/i18n'
import { formatSize } from '@/utils/format'
import type { OrphanEntry, OrphanDeleteItem, OrphanDeleteResult } from '@/types/mole'

/**
 * 残留清理入口面板（卸载列表尾部）：一体式卡片——头部行点击展开，
 * 内容在同一卡片内部出现（无第二层盒子），不离开列表。
 *
 * - 仅展示可清理项（deletable=true），受保护项不进列表；
 * - 默认全选，支持单点取消 / 全选切换，一键移入废纸篓（带原生确认框）；
 * - 与通知定向点入的全屏残留视图（OrphansTab）共享同一后端命令，
 *   这里是"顺手清一下"的轻量形态；展开态首次懒扫描，数据在组件存活期内保留。
 */
export function ResidualEntryPanel() {
  const tauri = useTauri()
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  /** null = 尚未扫描（含扫描失败态，可点击重试） */
  const [items, setItems] = useState<OrphanEntry[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [checkedPaths, setCheckedPaths] = useState<Set<string>>(new Set())
  /** 扫描防重入（快速开关/连点重试） */
  const scanningRef = useRef(false)

  const deletableItems = useMemo(() => (items ?? []).filter((o) => o.deletable), [items])
  const checkedCount = checkedPaths.size
  const allSelected = deletableItems.length > 0 && checkedCount === deletableItems.length
  const checkedSize = useMemo(
    () =>
      deletableItems
        .filter((o) => checkedPaths.has(o.path))
        .reduce((s, o) => s + o.size_bytes, 0),
    [deletableItems, checkedPaths]
  )

  /** 全量扫描孤儿残留（可清理性由后端 deletable 字段标定） */
  const scan = useCallback(async () => {
    if (scanningRef.current) return
    scanningRef.current = true
    setLoading(true)
    try {
      const result = (await tauri.mole_orphan_scan()) as OrphanEntry[]
      const list = result ?? []
      setItems(list)
      // 默认全选所有可清理项
      const next = new Set<string>()
      for (const o of list) {
        if (o.deletable) next.add(o.path)
      }
      setCheckedPaths(next)
    } catch (err) {
      console.error('[ResidualEntryPanel] scan failed', err)
      moleMessage.error(t('uninstall.orphan.scanFailed', { error: String(err) }))
    } finally {
      scanningRef.current = false
      setLoading(false)
    }
  }, [tauri, t])

  /** 展开/收起：首次展开懒扫描；失败态（items 仍为 null）再次展开会重试 */
  const handleToggleExpanded = useCallback(() => {
    const next = !expanded
    setExpanded(next)
    if (next && items === null && !scanningRef.current) void scan()
  }, [expanded, items, scan])

  const toggleCheck = useCallback((path: string) => {
    setCheckedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const toggleAll = useCallback(() => {
    if (deleting) return
    const paths = deletableItems.map((o) => o.path)
    setCheckedPaths((prev) => (prev.size === paths.length ? new Set() : new Set(paths)))
  }, [deletableItems, deleting])

  const handleDelete = useCallback(async () => {
    const targets = deletableItems.filter((o) => checkedPaths.has(o.path))
    if (targets.length === 0 || deleting || loading) return

    const confirmed = await moleNativeConfirm(t('uninstall.orphan.confirmTitle', { count: targets.length }), {
      informativeText: t('uninstall.orphan.confirmBody'),
      okLabel: t('uninstall.orphan.okMoveToTrash'),
      kind: 'warning',
    })
    if (!confirmed) return

    setDeleting(true)
    try {
      // 携带扫描已测体积：后端跳过 du 全树，释放量/日志与列表展示同口径
      const payload: OrphanDeleteItem[] = targets.map((o) => ({
        path: o.path,
        size_bytes: o.size_bytes,
      }))
      const result = (await tauri.mole_orphan_delete({ items: payload })) as OrphanDeleteResult
      const freed = formatSize(result.total_freed_bytes)
      if (result.failed_count > 0) {
        moleMessage.warning(
          t('uninstall.orphan.doneWithError', {
            success: result.success_count,
            failed: result.failed_count,
            freed,
          })
        )
      } else {
        moleMessage.success(t('uninstall.orphan.done', { count: result.success_count, freed }))
      }
      // 成功项从列表移除；失败项保留且勾选保留（可直接重试）
      const deletedSet = new Set(result.deleted_paths ?? [])
      setItems((prev) => (prev ? prev.filter((o) => !deletedSet.has(o.path)) : prev))
      setCheckedPaths((prev) => {
        const next = new Set<string>()
        for (const p of prev) {
          if (!deletedSet.has(p)) next.add(p)
        }
        return next
      })
    } catch (err) {
      console.error('[ResidualEntryPanel] delete failed', err)
      moleMessage.error(t('uninstall.orphan.deleteFailed', { error: String(err) }))
    } finally {
      setDeleting(false)
    }
  }, [deletableItems, checkedPaths, deleting, loading, tauri, t])

  return (
    <div className="mx-[24px] pt-2">
      {/* 一体式卡片：头部行点击展开，内容在同一卡片内部出现（无第二层盒子） */}
      <div className="rounded-lg border border-white/[0.10] bg-black/[0.22] overflow-hidden">
        {/* 头部行：点击展开/收起（箭头旋转 180° 指示状态；hover 微提亮） */}
        <div
          onClick={handleToggleExpanded}
          className="group flex items-center gap-3 px-4 py-3 cursor-pointer transition-colors hover:bg-white/[0.05]"
        >
          <span className="w-9 h-9 rounded-full bg-white/[0.06] flex items-center justify-center shrink-0">
            <Archive size={16} className="text-white/50" />
          </span>
          <div className="flex-1 min-w-0 select-none">
            <div className="text-xs text-white/80 font-medium">{t('uninstall.residualEntry.title')}</div>
            <div className="text-[10px] text-white/35 truncate mt-0.5">
              {t('uninstall.residualEntry.desc')}
            </div>
          </div>
          <ChevronDown
            size={15}
            className={`shrink-0 text-white/40 group-hover:text-white/70 transition-transform ${
              expanded ? 'rotate-180' : ''
            }`}
          />
        </div>

        {/* 展开内容：同一卡片内部，浅分隔线分界；仅列表可清理项 */}
        {expanded && (
          <div className="border-t border-white/[0.06] px-3 pt-2.5 pb-3">
            {loading && items === null ? (
              /* 首扫加载态 */
              <div className="flex items-center justify-center gap-2 py-4 text-[11px] text-white/40">
                <Loader2 size={13} className="animate-spin" />
                {t('uninstall.orphan.scanning')}
              </div>
            ) : items === null ? (
              /* 扫描失败态：点击重试 */
              <button
                onClick={() => void scan()}
                className="w-full py-3 text-center text-[11px] text-white/40 hover:text-white/70 transition-colors"
              >
                {t('uninstall.residualEntry.retry')}
              </button>
            ) : deletableItems.length === 0 ? (
              /* 无可清理项 */
              <div className="py-3 text-center text-[11px] text-white/35">
                {t('uninstall.residualEntry.empty')}
              </div>
            ) : (
              <>
                {/* 头部：已选计数 + 全选/取消全选 */}
                <div className="flex items-center justify-between px-1 pb-1.5">
                  <span className="text-[10px] text-white/45">
                    {t('uninstall.orphan.detailSelected', {
                      selected: checkedCount,
                      total: deletableItems.length,
                      size: formatSize(checkedSize),
                    })}
                  </span>
                  <button
                    onClick={toggleAll}
                    disabled={deleting}
                    className="text-[10px] text-white/45 hover:text-white/80 transition-colors disabled:pointer-events-none disabled:opacity-60"
                  >
                    {allSelected ? t('uninstall.orphan.selectNone') : t('uninstall.orphan.selectAll')}
                  </button>
                </div>

                {/* 条目列表（面板内滚动走全站 .mole-scroll 契约） */}
                <SimpleBar className="mole-scroll" style={{ maxHeight: 220 }}>
                  <div className="space-y-px pr-2">
                    {deletableItems.map((o) => (
                      <div
                        key={o.path}
                        onClick={() => toggleCheck(o.path)}
                        className={`flex items-center gap-2.5 px-2.5 py-1.5 rounded-md cursor-pointer transition-colors select-none hover:bg-white/[0.05] ${
                          deleting ? 'pointer-events-none' : ''
                        }`}
                      >
                        {/* 勾选框：stopPropagation 防冒泡到行 onClick 造成双 toggle 抵消 */}
                        <MoleCheckbox
                          checked={checkedPaths.has(o.path)}
                          onClick={(e) => {
                            e.stopPropagation()
                            toggleCheck(o.path)
                          }}
                        />
                        <div className="flex-1 min-w-0">
                          <div className="text-[11px] text-white/85 truncate" title={o.file_name}>
                            {o.file_name}
                          </div>
                          <div className="text-[9px] text-white/30 truncate font-mono" title={o.path}>
                            {o.path}
                          </div>
                        </div>
                        <span className="text-[10px] text-white/45 font-mono shrink-0">
                          {o.size_human}
                        </span>
                      </div>
                    ))}
                  </div>
                </SimpleBar>

                {/* 底部：主操作 */}
                <div className="flex items-center justify-end px-1 pt-2">
                  <button
                    onClick={handleDelete}
                    disabled={deleting || loading || checkedCount === 0}
                    className="apps-primary-btn text-[11px] px-3 py-1 rounded-md font-bold disabled:opacity-50 flex items-center gap-1.5"
                  >
                    {deleting ? <Loader2 size={12} className="animate-spin" /> : <Trash2 size={12} />}
                    {deleting
                      ? t('uninstall.orphan.deleting')
                      : allSelected || checkedCount === 0
                        ? t('uninstall.orphan.moveAll')
                        : t('uninstall.orphan.moveSelected', { count: checkedCount })}
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
