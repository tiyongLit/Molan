import { useState, useCallback, useEffect, useMemo, useLayoutEffect, useRef, memo } from 'react'
import { createPortal } from 'react-dom'
import { Trash2, RefreshCw, Loader2, ShieldAlert, CheckCircle2 } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import useTauri from '@/hooks/useTauri'
import { moleMessage, MoleCheckbox, ExpandChevron } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { useResidual } from '@/layout/ResidualContext'
import { useI18n } from '@/i18n'
import { formatSize } from '@/utils/format'
import type {
  OrphanEntry,
  OrphanDeleteItem,
  OrphanDeleteResult,
  ResidualTarget,
} from '@/types/mole'

/**
 * 残留孤儿：总览卡 + 按需明细的「行动型」页面（按用户行为推导的展示方式，
 * 替代早期的分类分组浏览——用户来这里只为"清掉"，浏览/研究是低频次要行为）。
 *
 * - 总览卡：数量/体积 + 唯一主动作（全部移入废纸篓）+ 明细开关 + 保护项透明提示；
 * - 明细：平铺列表，仅含可清理项，默认全选；勾选只服务"想留几项"的例外场景；
 * - 不可清理项（受系统保护的持久状态）不进列表，仅一行汇总 + 原生弹窗说明。
 */

// ── 行组件（memo）：明细列表的一行，只渲染可清理项 ──

interface OrphanRowProps {
  orphan: OrphanEntry
  checked: boolean
  /** 删除中：整行关闭交互（含 hover） */
  disabled: boolean
  onToggle: (path: string) => void
}

const OrphanRow = memo(function OrphanRow({ orphan, checked, disabled, onToggle }: OrphanRowProps) {
  return (
    <div
      className={`flex items-center gap-2.5 px-3 py-2 rounded-md cursor-pointer transition-colors select-none hover:bg-white/[0.06] ${
        disabled ? 'pointer-events-none' : ''
      }`}
      onClick={() => onToggle(orphan.path)}
    >
      {/* 勾选框：stopPropagation 防冒泡到行 onClick 造成双 toggle 抵消 */}
      <MoleCheckbox
        checked={checked}
        onClick={(e) => {
          e.stopPropagation()
          onToggle(orphan.path)
        }}
      />

      {/* 文件名 + 路径（截断时悬停 title 查看完整值） */}
      <div className="flex-1 min-w-0">
        <div className="text-xs text-white/90 truncate font-medium" title={orphan.file_name}>
          {orphan.file_name}
        </div>
        <div className="text-[10px] text-white/35 truncate font-mono" title={orphan.path}>
          {orphan.path}
        </div>
      </div>

      {/* 大小 */}
      <span className="text-[11px] text-white/50 font-mono shrink-0">{orphan.size_human}</span>
    </div>
  )
})

// memo 包装：壳层 hidden 常驻挂载下，父级状态变化（切 tab 等）不触发子树 diff
export const OrphansTab = memo(function OrphansTab({ active = true }: { active?: boolean }) {
  const tauri = useTauri()
  const { t } = useI18n()
  const { residualTarget, clearResidualTarget } = useResidual()
  const [orphans, setOrphans] = useState<OrphanEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [scanned, setScanned] = useState(false)
  const [checkedPaths, setCheckedPaths] = useState<Set<string>>(new Set())
  const [deleting, setDeleting] = useState(false)
  /** 明细区展开状态：总览卡常驻，明细（列表）按需展开 */
  const [detailOpen, setDetailOpen] = useState(false)
  /** 明细列表容器：扫描结果应用后经 closest 找到 SimpleBar 滚动元素并归零 */
  const listRef = useRef<HTMLDivElement | null>(null)
  /** 扫描防重入：in-flight 中再次触发则记录待补跑目标（后到者胜），完成后自动接续 */
  const scanningRef = useRef(false)
  const queuedScanRef = useRef<{ target: ResidualTarget | null } | null>(null)
  /** 上次扫描的可清理 path 集：重扫时区分"新增项（默认勾选）"与"用户取消过的旧项（保持取消）" */
  const prevPathsRef = useRef<Set<string>>(new Set())

  // 扫描按钮组通过 Portal 渲染到壳层顶行右上角（与卸载页工具栏同一行），
  // 状态与逻辑仍留在本组件，Portal 保留 React 树，事件/上下文不受影响。
  // 壳层用 hidden 切换常驻本组件：tab 重新激活时槽位是新建的 DOM，需重取。
  const [toolbarSlot, setToolbarSlot] = useState<HTMLElement | null>(null)
  useLayoutEffect(() => {
    if (!active) {
      setToolbarSlot(null)
      return
    }
    setToolbarSlot(document.getElementById('orphans-toolbar-slot'))
  }, [active])

  // ── 扫描 ──
  // 全量（mole_orphan_scan）与定向（mole_orphan_scan_for）共用结果应用逻辑
  const applyScanResult = useCallback((result: OrphanEntry[] | null | undefined) => {
    const list = result ?? []
    setOrphans(list)
    setScanned(true)
    // 勾选策略：首次扫描默认全选；重扫保留每个 path 的既有勾选状态
    // （用户取消过的旧项保持取消），新出现的可清理项默认勾选——
    // "用户调整过的事不要让他调第二次"
    const nextPaths = new Set<string>()
    for (const o of list) {
      if (o.deletable) nextPaths.add(o.path)
    }
    const prevPaths = prevPathsRef.current
    prevPathsRef.current = nextPaths
    setCheckedPaths((prev) => {
      const next = new Set<string>()
      for (const p of nextPaths) {
        if (!prevPaths.has(p) || prev.has(p)) next.add(p)
      }
      return next
    })
    // 新结果回到列表顶部：旧滚动位置对全新数据没有意义
    listRef.current?.closest<HTMLElement>('.simplebar-content-wrapper')?.scrollTo({ top: 0 })
  }, [])

  /** 取出并清空「待补跑」队列（原子读；独立函数以避开 TS 对 ref.current 的控制流窄化） */
  const takeQueuedTarget = useCallback(() => {
    const queued = queuedScanRef.current
    queuedScanRef.current = null
    return queued
  }, [])

  /** 执行扫描：target 非空 → 定向（仅目标 app 的残留）；null → 全量。 */
  const runScan = useCallback(
    async (target: ResidualTarget | null) => {
      // stale-while-revalidate：重扫不清旧列表/旧勾选，避免整页闪 spinner；
      // 全屏 spinner 仅由渲染逻辑在「无数据可保留」时展示（首扫或上次结果为空）。
      // 防重入：扫描中再次触发（如定向扫描中途点「查看全部残留」）→
      // 记录待补跑目标（后到者胜），当前扫描完成后自动接续，避免并发互相覆盖
      if (scanningRef.current) {
        queuedScanRef.current = { target }
        return
      }
      scanningRef.current = true
      setLoading(true)
      try {
        let next: ResidualTarget | null = target
        for (;;) {
          try {
            const result = (next
              ? await tauri.mole_orphan_scan_for({
                  bundle_id: next.bundleId,
                  app_name: next.appName,
                })
              : await tauri.mole_orphan_scan()) as OrphanEntry[]
            applyScanResult(result)
          } catch (err) {
            console.error('[OrphansTab] scan failed', err)
            moleMessage.error(t('uninstall.orphan.scanFailed', { error: String(err) }))
          }
          const queued = takeQueuedTarget()
          if (queued === null) break
          next = queued.target
        }
      } finally {
        scanningRef.current = false
        setLoading(false)
      }
    },
    [tauri, t, applyScanResult, takeQueuedTarget],
  )

  // 定向目标变化：非空 → 自动定向扫描；被清除（「查看全部残留」）→ 回全量扫描。
  // 同一目标 + 依赖身份变化（runScan 重建）不重扫，仅响应目标本身的变化。
  const prevTargetRef = useRef<ResidualTarget | null>(null)
  useEffect(() => {
    const prev = prevTargetRef.current
    prevTargetRef.current = residualTarget
    if (residualTarget === prev) return
    if (residualTarget) {
      void runScan(residualTarget)
    } else if (prev) {
      void runScan(null)
    }
  }, [residualTarget, runScan])

  // 进入残留视图且从未扫描过 → 自动开始全量扫描（意图明确，省掉"点一下扫描"）。
  // 定向目标由上方 effect 全权负责，这里不重复触发；
  // triedRef 防"扫描失败 → loading 回落 → 反复自动重试"（失败后用户可手动重扫）。
  const autoScanTriedRef = useRef(false)
  useEffect(() => {
    if (!active || scanned || loading || residualTarget) return
    if (autoScanTriedRef.current) return
    autoScanTriedRef.current = true
    void runScan(null)
  }, [active, scanned, loading, residualTarget, runScan])

  // ── 删除 ──
  const handleDelete = useCallback(async () => {
    const targets = orphans.filter((o) => checkedPaths.has(o.path))
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
      const items: OrphanDeleteItem[] = targets.map((o) => ({
        path: o.path,
        size_bytes: o.size_bytes,
      }))
      const result = (await tauri.mole_orphan_delete({ items })) as OrphanDeleteResult
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
      // 从列表中移除已删除项：按后端逐条结果精确剔除（失败项保留在列表）
      const deletedSet = new Set(result.deleted_paths ?? [])
      setOrphans((prev) => prev.filter((o) => !deletedSet.has(o.path)))
      // 失败项保留勾选（用户可直接重试），成功项清掉
      setCheckedPaths((prev) => {
        const next = new Set<string>()
        for (const p of prev) {
          if (!deletedSet.has(p)) next.add(p)
        }
        return next
      })
    } catch (err) {
      console.error('[OrphansTab] delete failed', err)
      moleMessage.error(t('uninstall.orphan.deleteFailed', { error: String(err) }))
    } finally {
      setDeleting(false)
    }
  }, [orphans, checkedPaths, deleting, loading, tauri, t])

  // ── 勾选 ──
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
    const deletablePaths = orphans.filter((o) => o.deletable).map((o) => o.path)
    setCheckedPaths((prev) => {
      if (prev.size === deletablePaths.length) return new Set()
      return new Set(deletablePaths)
    })
  }, [orphans, deleting])

  // ── 派生数据 ──
  const deletableOrphans = useMemo(() => orphans.filter((o) => o.deletable), [orphans])
  const deletableCount = deletableOrphans.length
  const protectedCount = orphans.length - deletableCount
  const checkedCount = checkedPaths.size
  const allSelected = deletableCount > 0 && checkedCount === deletableCount
  const totalSize = useMemo(() => orphans.reduce((s, o) => s + o.size_bytes, 0), [orphans])
  const deletableSize = useMemo(
    () => deletableOrphans.reduce((s, o) => s + o.size_bytes, 0),
    [deletableOrphans]
  )
  /** 受保护项总体积（保护说明弹窗用） */
  const protectedSize = totalSize - deletableSize
  const checkedSize = useMemo(
    () =>
      deletableOrphans
        .filter((o) => checkedPaths.has(o.path))
        .reduce((s, o) => s + o.size_bytes, 0),
    [deletableOrphans, checkedPaths]
  )

  /** 保护项说明（单按钮原生提示；含总体积，让"跳过"更有依据） */
  const showProtectedInfo = useCallback(() => {
    void moleNativeConfirm(t('uninstall.orphan.protectedTitle'), {
      informativeText: t('uninstall.orphan.protectedBody', {
        count: protectedCount,
        size: formatSize(protectedSize),
      }),
      kind: 'info',
      okLabel: t('common.gotIt'),
      cancelLabel: null, // 单按钮提示模式
    })
  }, [t, protectedCount, protectedSize])

  return (
    <div className="h-full flex flex-col">
      {/* ── 总览卡（扫描后常驻，不随明细滚动）──
          行为依据：用户的核心动作是"清掉"，这里承担 数量/体积 → 安心说明 → 唯一主动作 */}
      {scanned && orphans.length > 0 && (
        <div className="shrink-0 px-6 pt-4 pb-3">
          {/* 主信息：可清理数量 + 体积（行动口径；受保护项由下方跳过提示承载） */}
          <div className="text-[13px] text-white/90">
            {deletableCount > 0 ? (
              <>
                {t('uninstall.orphan.summary.title', { count: deletableCount })}
                {' · '}
                <span className="font-mono text-white">{formatSize(deletableSize)}</span>
              </>
            ) : (
              t('uninstall.orphan.summary.noneDeletable')
            )}
          </div>

          {/* 副行：全量 → 安心说明；定向 → 仅显示目标提示 + 查看全部 */}
          <div className="flex items-center gap-2 mt-1 min-w-0">
            {residualTarget ? (
              <>
                <span className="text-[11px] text-emerald-200/85 truncate">
                  {t('uninstall.orphan.targetedBanner', { app: residualTarget.appName })}
                </span>
                <button
                  onClick={clearResidualTarget}
                  disabled={loading || deleting}
                  className="shrink-0 rounded px-2 py-0.5 text-[10px] font-medium text-emerald-300 bg-emerald-500/15 hover:bg-emerald-500/25 transition-colors disabled:pointer-events-none disabled:opacity-60"
                >
                  {t('uninstall.orphan.showAll')}
                </button>
              </>
            ) : (
              <span className="text-[11px] text-white/45 truncate">
                {t('uninstall.orphan.summary.desc')}
              </span>
            )}
          </div>

          {/* 行动行：唯一主动作 + 明细开关 */}
          <div className="flex items-center gap-3 mt-3">
            <button
              onClick={handleDelete}
              disabled={deleting || loading || checkedCount === 0}
              className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold disabled:opacity-50 flex items-center gap-1.5"
            >
              {deleting ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
              {deleting
                ? t('uninstall.orphan.deleting')
                : allSelected || checkedCount === 0
                  ? t('uninstall.orphan.moveAll')
                  : t('uninstall.orphan.moveSelected', { count: checkedCount })}
            </button>

            {deletableCount > 0 && (
              <button
                onClick={() => setDetailOpen((v) => !v)}
                className="flex items-center gap-1 text-xs text-white/70 hover:text-white transition-colors"
              >
                <ExpandChevron expanded={detailOpen} />
                {detailOpen
                  ? t('uninstall.orphan.detailHide')
                  : t('uninstall.orphan.detailShow', { count: deletableCount })}
              </button>
            )}
          </div>

          {/* 保护项：一句话透明（点击看说明），不占列表 */}
          {protectedCount > 0 && (
            <button
              onClick={showProtectedInfo}
              className="mt-2 text-[10px] text-white/35 hover:text-white/60 transition-colors"
            >
              {t('uninstall.orphan.protectedNote', { count: protectedCount })}
            </button>
          )}
        </div>
      )}

      {/* 扫描完成后才出现：重新扫描，Portal 到壳层顶行右上角（对齐卸载页工具栏） */}
      {toolbarSlot &&
        scanned &&
        createPortal(
          <div className="flex items-stretch h-7 rounded-[3px] bg-black/[0.3] border border-white/[0.14] overflow-hidden">
            {/* 重扫按钮：loading 中图标转圈 + 禁用（stale 列表保留时唯一的刷新反馈） */}
            <button
              onClick={() => void runScan(residualTarget)}
              disabled={loading || deleting}
              title={t('uninstall.rescan')}
              className="group flex items-center gap-1.5 px-2.5 text-xs text-white transition-colors hover:bg-white/[0.08] disabled:pointer-events-none disabled:opacity-60"
            >
              <RefreshCw
                size={13}
                className={
                  loading
                    ? 'animate-spin opacity-70'
                    : 'opacity-70 group-hover:opacity-100 transition-opacity'
                }
              />
              <span className="opacity-85 group-hover:opacity-100 transition-opacity">{t('uninstall.rescan')}</span>
            </button>
          </div>,
          toolbarSlot
        )}

      {/* ── 明细区（按需展开）：轻量头部 + 平铺列表（仅可清理项）──
          行为依据：明细是少数人的"确认/例外"入口，默认不占据首屏 */}
      {scanned && orphans.length > 0 && detailOpen && deletableCount > 0 && (
        <>
          <div className="shrink-0 flex items-center justify-between px-6 pb-1.5">
            <span className="text-[10px] text-white/45">
              {t('uninstall.orphan.detailSelected', {
                selected: checkedCount,
                total: deletableCount,
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

          <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
            <div ref={listRef} className="px-6 pb-4">
              <div className="space-y-px">
                {deletableOrphans.map((orphan) => (
                  <OrphanRow
                    key={orphan.path}
                    orphan={orphan}
                    checked={checkedPaths.has(orphan.path)}
                    disabled={deleting}
                    onToggle={toggleCheck}
                  />
                ))}
              </div>
            </div>
          </SimpleBar>
        </>
      )}

      {/* 初始态：尚未扫描（唯一扫描入口；扫描后由右上角 Portal 的「重新扫描」接管） */}
      {!scanned && !loading && (
        <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-4 text-white/40">
          <ShieldAlert size={40} strokeWidth={1.2} />
          <div className="flex flex-col items-center gap-1.5">
            <p className="text-sm text-white/70">{t('uninstall.orphan.empty.title')}</p>
            <p className="text-xs text-white/25 max-w-xs text-center">
              {t('uninstall.orphan.empty.desc')}
            </p>
          </div>
          <button
            onClick={() => void runScan(residualTarget)}
            className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold flex items-center gap-1.5"
          >
            <RefreshCw size={13} />
            {t('uninstall.orphan.scan')}
          </button>
        </div>
      )}

      {/* 全屏 spinner 仅在无可保留数据时出现（首扫或上次结果为空）：
          有旧列表的重扫走 stale 模式，列表原地保留，刷新反馈在右上角重扫按钮 */}
      {loading && orphans.length === 0 && (
        <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-3 text-white/40">
          <Loader2 size={32} className="animate-spin" />
          <p className="text-sm">{t('uninstall.orphan.scanning')}</p>
        </div>
      )}

      {/* 扫描完成且无结果 */}
      {scanned && !loading && orphans.length === 0 && (
        <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-3 text-white/40">
          <CheckCircle2 size={40} strokeWidth={1.2} className="text-emerald-400/60" />
          {residualTarget ? (
            <>
              {/* 定向未命中：区别于全量干净的文案 + 查看全部入口 */}
              <p className="text-sm">
                {t('uninstall.orphan.targetedEmpty', { app: residualTarget.appName })}
              </p>
              <button
                onClick={clearResidualTarget}
                className="apps-ghost-btn text-xs px-3 py-1.5 rounded-md transition-colors mt-1"
              >
                {t('uninstall.orphan.showAll')}
              </button>
            </>
          ) : (
            <>
              <p className="text-sm">{t('uninstall.orphan.none')}</p>
              <p className="text-xs text-white/25">{t('uninstall.orphan.clean')}</p>
            </>
          )}
        </div>
      )}
    </div>
  )
})
