import { useEffect, useMemo, useRef, useState, useCallback } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import { load, type Store } from '@tauri-apps/plugin-store'
import CleanStatusBar, { type CleanStatus } from './components/CleanStatusBar'
import { ScanningGroupRow } from './components/ScanningSkeletonList'
import ScanResult from './components/ScanResult'
import CategoryRow from './components/CategoryRow'
import CleanFooter from './components/CleanFooter'
import useTauri from '@/hooks/useTauri'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { moleMessage, ScanPageLayout, type ScanPageActionConfig } from '@/components/ui'
import './Clean.scss'

import type { MoleCleanResult, MoleCleanItem, CleanStatusInfo } from '@/types/mole'
import { useI18n } from '@/i18n'
import { CATEGORY_GROUPS, PRIMARY_CTA_STYLE, type CleanGroupData } from './clean.constants'
import { isCountableCleanItem, selKey, computeGroupStatusBySection } from './scan-status'
import { useScanEngine } from './hooks/useScanEngine'
import { useCleanEngine } from './hooks/useCleanEngine'
import { useCleanJob } from './hooks/useCleanJob'
import { useSelectionPersistence } from './hooks/useSelectionPersistence'
import { uiTrace } from '@/utils/uiTrace'

// ============================================================
// 页面阶段：火绒式 idle → scanning → review → cleaning → done
// scanning 由后端任务快照派生（唯一事实来源）；其余为前端视图阶段
// ============================================================
type Phase = 'idle' | 'scanning' | 'review' | 'cleaning' | 'done'
type ViewStage = Exclude<Phase, 'scanning'>

// ============================================================
// 主组件
// 状态分层：
//  - 页面级：phase / scanResult / scanError / doneSummary（视图切换与结果展示）
//  - useScanEngine：扫描进度（cleanup::phase-result 事件驱动）
//  - useCleanEngine：清理进度 + 动画队列 + 后端调用
//  - useSelectionPersistence：勾选状态 + store 偏好持久化
// ============================================================
export function Clean() {
  const navigate = useNavigate()
  const location = useLocation()
  const tauri = useTauri()
  const { t } = useI18n()
  const [executionAllowed, setExecutionAllowed] = useState(false)
  useEffect(() => {
    let stale = false
    tauri.clean_status().then((status: CleanStatusInfo) => {
      if (!stale) setExecutionAllowed(status.execution_allowed === true)
    }).catch(() => { if (!stale) setExecutionAllowed(false) })
    return () => { stale = true }
  }, [tauri])

  // 时序埋点（卡顿分析）：页面挂载/卸载时间点
  useEffect(() => {
    uiTrace('clean.mount')
    return () => { uiTrace('clean.unmount') }
  }, [])

  // 视图阶段（不含 scanning）：scanning 由后端任务快照派生，前端不做本地猜测
  const [stage, setStage] = useState<ViewStage>('idle')
  const [scanResult, setScanResult] = useState<MoleCleanResult | null>(null)
  const [scanError, setScanError] = useState<string | null>(null)
  // 扫描收尾中：后端已回到 idle、结果正在取回（保持 scanning 视图，避免闪一帧 idle）
  const [consumingScan, setConsumingScan] = useState(false)

  const [expandedCategories, setExpandedCategories] = useState<Set<string>>(new Set())
  const [doneSummary, setDoneSummary] = useState<{ totalCleaned: number; failedCount: number; permanentDelete: boolean } | null>(null)

  // ---- 删除模式（直接删除 vs 废纸篓）+ store 持久化 ----
  const [permanentDelete, setPermanentDelete] = useState(true) // 默认直接删除
  const prefStoreRef = useRef<Store | null>(null)
  useEffect(() => {
    let cancelled = false
    load('clean-preferences.bin').then((store) => {
      if (cancelled) return
      prefStoreRef.current = store
      store.get<boolean>('permanentDelete').then((saved) => {
        if (!cancelled && saved !== undefined && saved !== null) {
          setPermanentDelete(saved)
        }
      }).catch(() => { /* 首次使用无此 key，保持默认 */ })
    })
    return () => { cancelled = true }
  }, [])
  const handlePermanentDeleteChange = useCallback((value: boolean) => {
    setPermanentDelete(value)
    const store = prefStoreRef.current
    if (store) {
      store.set('permanentDelete', value).then(() => store.save()).catch(() => {})
    }
  }, [])

  // ---- Clean 任务状态机投影（后端唯一事实来源）：scanning 由快照派生，前端不做本地猜测 ----
  const {
    snapshot: jobSnapshot,
    isScanActive,
    finishedJobId,
    ackFinished,
    startScan: startScanJob,
    cancelScan: cancelScanJob,
    fetchResult: fetchScanResult,
  } = useCleanJob()
  // 收尾期间（结果取回中）保持 scanning 视图，避免 review 前闪一帧 idle
  const scanLive = isScanActive || Boolean(finishedJobId) || consumingScan
  const isAuthorizing = jobSnapshot?.state === 'authorizing'
  const cancelling = jobSnapshot?.state === 'cancelling'
  // phase 为派生值：scanning 由后端快照驱动，其余为本地视图阶段
  const phase: Phase = scanLive ? 'scanning' : stage

  // 时序埋点（卡顿分析）：任务快照每次迁移打点（同一 state#seq 去重）
  const lastTracedStateRef = useRef<string>('')
  useEffect(() => {
    const s = jobSnapshot?.state ?? 'none'
    const key = `${s}#${jobSnapshot?.seq ?? -1}`
    if (lastTracedStateRef.current === key) return
    lastTracedStateRef.current = key
    uiTrace('clean.jobstate', `state=${s} seq=${jobSnapshot?.seq ?? -1} auth=${jobSnapshot?.auth ?? '-'} job=${jobSnapshot?.job_id ?? '-'}`)
  }, [jobSnapshot])

  // 扫描进度（由 cleanup::phase-result 事件驱动）
  const {
    scanTarget,
    scanProgress,
    accumulatedSizeKb,
    scanCompletedSections,
    streamedCategories,
    reset: resetScan,
    complete: completeScan,
  } = useScanEngine(scanLive)

  // 清理进度 + 动画队列（由 clean::apply-progress 事件驱动）
  const {
    cleanProgress,
    cleanCurrent,
    cleanedItemKeys,
    prepare: prepareClean,
    applyClean,
    cancelClean: cancelCleanEngine,
  } = useCleanEngine(stage === 'cleaning')

  // ---- 扫描任务结束 → 取结果切视图（每个任务恰消费一次；取消/无结果 → 回 idle）----
  useEffect(() => {
    if (!finishedJobId) { setConsumingScan(false); return }
    let stale = false
    setConsumingScan(true)
    // 时序埋点（卡顿分析）：结果取回耗时（结论落 review/idle 前必经的一步）
    uiTrace('clean.result.fetch', `begin job=${finishedJobId}`)
    const tFetch = performance.now()
    ;(async () => {
      let nextStage: ViewStage = 'idle'
      try {
        const result = (await fetchScanResult(finishedJobId)) as MoleCleanResult
        if (!result?.cancelled) {
          if (!stale) {
            setScanResult(result)
            completeScan()
          }
          nextStage = 'review'
        }
      } catch {
        // 结果不可得（授权期间取消 / worker 异常）：静默回 idle
        nextStage = 'idle'
      }
      if (stale) return
      if (nextStage === 'idle') setScanResult(null)
      setStage(nextStage)
      setConsumingScan(false)
      ackFinished()
      uiTrace('clean.result.fetch', `done stage=${nextStage} took=${(performance.now() - tFetch).toFixed(1)}ms`)
    })()
    return () => { stale = true }
  }, [finishedJobId, fetchScanResult, completeScan, ackFinished])

  // ---- 授权失败提示（每任务一次；用户取消授权保持静默，沿用受限扫描）----
  const authWarnedJobRef = useRef<string | null>(null)
  useEffect(() => {
    const jobId = jobSnapshot?.job_id
    if (!jobId || jobSnapshot?.kind !== 'scan' || jobSnapshot.auth !== 'failed') return
    if (authWarnedJobRef.current === jobId) return
    authWarnedJobRef.current = jobId
    moleMessage.warning(t('clean.authFailed'))
  }, [jobSnapshot, t])

  // 勾选状态 + store 偏好持久化
  const {
    selectedItemIds,
    resetForNewScan,
    initializeFromScan,
    toggleItem,
    toggleGroup: toggleGroupSelection,
    resetToDefault: handleResetToDefault,
    hasChangedFromDefault,
    persist,
  } = useSelectionPersistence()

  // 渐进式扫描：scanning 阶段用逐段到达的 streamedCategories 渲染真列表；
  // 其余阶段以最终 clean_scan 返回的 scanResult 为权威（含 scan_id，供 apply 校验）。
  const allCategories = phase === 'scanning'
    ? streamedCategories
    : (scanResult?.categories || [])

  const flatItems = useMemo(() => {
    const result: MoleCleanItem[] = []
    for (const cat of allCategories) {
      for (const item of cat.items) {
        result.push({
          ...item,
          categoryId: item.categoryId || cat.id,
          categoryTitle: item.categoryTitle || cat.title,
          recommend: item.recommend ?? cat.recommend,
          cautious: item.cautious ?? cat.cautious,
        })
      }
    }
    return result
  }, [allCategories])

  // ---- 扫描完成后初始化勾选：后端默认 → store 偏好覆盖（initializeFromScan 内部守门）----
  useEffect(() => {
    if (stage !== 'review' || !scanResult?.categories) return
    initializeFromScan(scanResult.categories)
  }, [stage, scanResult, initializeFromScan])

  // ---- 扫描完成后默认展开所有分类 ----
  useEffect(() => {
    if (stage !== 'review') return
    setExpandedCategories(new Set(CATEGORY_GROUPS.map(g => g.id)))
  }, [stage])

  // ---- 启动扫描：只向后端表达意图，后续状态全部由 job-state 快照驱动 ----
  const startScan = useCallback(async () => {
    // 时序埋点（卡顿分析）：向后端表达扫描意图的时间点
    uiTrace('clean.scan.start', 'metric=logical')
    setScanError(null)
    setScanResult(null)
    // 重置勾选初始化标记，保证下次扫描完成后重新从 store 加载偏好
    resetForNewScan()
    resetScan()
    setStage('idle')
    try {
      // 立即受理并广播（authorizing → scanning）：UI 在下一次快照即切换，
      // 不再等授权面板结果——这是「后端在扫、前端仍 idle」分叉的根治点。
      await startScanJob('logical')
    } catch (e) {
      console.error('[Clean] clean_job_start failed', e)
      setScanError(typeof e === 'string' ? e : (e as Error)?.message || t('clean.error.scanFailed'))
    }
  }, [startScanJob, resetForNewScan, resetScan, t])

  // ---- 从 Home 一键跳转自动启动扫描 ----
  // 仅 Home 页 DashboardDiskCard 下方的「立即扫描」会带 state.autoScan=true 跳转；侧边栏 Link 直接进入不携带 state，不会触发。
  // 用 ref 守门防止 React.StrictMode 双跑 / 后退再次进入重复触发。
  const autoScanConsumedRef = useRef(false)
  useEffect(() => {
    if (autoScanConsumedRef.current) return
    const state = location.state as { autoScan?: boolean } | null
    if (!state?.autoScan) return
    autoScanConsumedRef.current = true
    // 时序埋点（卡顿分析）：autoScan 触发点
    uiTrace('clean.autoscan', 'fired')
    // 清空 state 避免刷新页面 / 后退再进入时重复触发
    navigate(location.pathname, { replace: true, state: null })
    startScan()
  }, [location.state, location.pathname, navigate, startScan])

  // ---- 取消扫描：向后端表达取消意图；UI 显示「正在取消...」，直到后端把状态置回 idle ----
  const cancelScan = useCallback(async () => {
    await cancelScanJob(jobSnapshot?.job_id)
  }, [cancelScanJob, jobSnapshot?.job_id])

  // ---- 按需加载图标 ----
  const loadedPathsRef = useRef<Set<string>>(new Set())

  const loadIconsForItems = useCallback((items: MoleCleanItem[]) => {
    const paths = items
      .map((i) => i.path)
      .filter((p): p is string => Boolean(p) && !loadedPathsRef.current.has(p))
    if (paths.length === 0) return
    for (const p of paths) loadedPathsRef.current.add(p)
  }, [])

  const groups = useMemo(() => {
    const result = CATEGORY_GROUPS.map((g) => {
      const items = flatItems.filter((i) => g.categoryIds.includes(i.categoryId))
      const countableItems = items.filter(isCountableCleanItem)
      const totalSize = countableItems.reduce((s, i) => s + (i.size || 0), 0)
      const selectedSize = items
        .filter((i) => isCountableCleanItem(i) && selectedItemIds.has(selKey(i.categoryId, i.id)))
        .reduce((s, i) => s + (i.size || 0), 0)
      const selectedCount = items.filter((i) => selectedItemIds.has(selKey(i.categoryId, i.id))).length
      const itemCount = items.length
      return { ...g, items, totalSize, selectedSize, selectedCount, itemCount }
    })
    return result
  }, [flatItems, selectedItemIds])

  const totalSize = useMemo(() => groups.reduce((s, g) => s + g.totalSize, 0), [groups])
  const totalSelectedSize = useMemo(() => groups.reduce((s, g) => s + g.selectedSize, 0), [groups])

  // 状态条展示状态：扫描完成未勾选 → ready；已勾选 → selected
  const displayStatus: CleanStatus = totalSelectedSize > 0 ? 'selected' : 'ready'

  // ---- 交互 ----
  const toggleCategory = useCallback((groupId: string, items: MoleCleanItem[]) => {
    setExpandedCategories((prev) => {
      const next = new Set(prev)
      if (next.has(groupId)) {
        next.delete(groupId)
      } else {
        next.add(groupId)
        loadIconsForItems(items)
      }
      return next
    })
  }, [loadIconsForItems])

  const toggleGroup = useCallback((g: CleanGroupData) => {
    const allSelected = g.itemCount > 0 && g.selectedCount === g.itemCount
    toggleGroupSelection(g.items, allSelected)
  }, [toggleGroupSelection])

  // ---- 立即清理 ----
  const handleClean = useCallback(async () => {
    if (!executionAllowed) {
      moleMessage.warning(t('clean.executionBlocked'))
      return
    }
    try {
      const status = await tauri.clean_status() as CleanStatusInfo
      if (!status.execution_allowed) throw new Error('CLEAN_EXECUTION_BLOCKED')
    } catch {
      setExecutionAllowed(false)
      moleMessage.warning(t('clean.executionBlocked'))
      return
    }
    if (totalSelectedSize === 0) return
    // 复位进度/动画标记（弹窗判定前，避免 review 阶段残留上次进度条）
    prepareClean()
    setDoneSummary(null)

    // 比对当前勾选 vs 后端默认勾选，有变化则弹窗询问是否记住
    if (hasChangedFromDefault()) {
      const shouldSave = await moleNativeConfirm(t('clean.confirm.savePrefs'), {
        informativeText: t('clean.confirm.savePrefsDetail'),
        kind: 'info',
        okLabel: t('clean.confirm.yes'),
        cancelLabel: t('clean.confirm.no'),
      })
      if (shouldSave) {
        // 保存当前勾选偏好到 store
        await persist(scanResult?.categories || [])
      }
    }

    // 构建清理队列（从 selectedItemIds 中筛选出所有选中的项）
    const queue = flatItems
      .filter((i) => selectedItemIds.has(selKey(i.categoryId, i.id)))
      .map((i) => ({ key: selKey(i.categoryId, i.id), item: i }))

    const scanId = scanResult?.scan_id || ''
    if (!scanId) {
      setStage('review') // 无 scan_id 说明扫描数据异常，回退到 review
      return
    }

    setStage('cleaning')
    // 动画队列 + 后端调用（useCleanEngine 内部并行执行）
    const outcome = await applyClean(queue, scanId, permanentDelete)
    if (!outcome.ok) {
      setScanError(outcome.error)
      setStage('review')
      return
    }
    setDoneSummary({ totalCleaned: outcome.totalCleaned, failedCount: outcome.failedCount, permanentDelete })
    setStage('done')
  }, [executionAllowed, tauri, totalSelectedSize, flatItems, selectedItemIds, scanResult, prepareClean, hasChangedFromDefault, persist, applyClean, permanentDelete, t])

  // ---- 取消清理 ----
  const cancelClean = useCallback(async () => {
    await cancelCleanEngine()
    setStage('review')
  }, [cancelCleanEngine])

  // ---- 返回初始态（review 状态条「返回」按钮）----
  const handleBackToIdle = useCallback(() => {
    resetForNewScan()
    setScanResult(null)
    setDoneSummary(null)
    setScanError(null)
    setStage('idle')
  }, [resetForNewScan])

  // ---- 完成：回主页 ----
  const handleFinish = useCallback(() => {
    navigate('/home')
  }, [navigate])

  // ---- 在 Finder 中显示 ----
  const handleReveal = useCallback(async (path: string) => {
    try {
      await tauri.clean_reveal_in_finder({ path })
    } catch (err) {
      console.error('[CleanItem] reveal in finder failed', err)
    }
  }, [tauri])

  // ============================
  // done 视图
  // ============================
  if (phase === 'done') {
    return (
      <ScanResult
        doneSummary={doneSummary}
        scanError={scanError}
        permanentDelete={doneSummary?.permanentDelete ?? true}
        onRescan={() => {
          resetForNewScan()
          setScanResult(null)
          setDoneSummary(null)
          setScanError(null)
          startScan()
        }}
        onFinish={handleFinish}
      />
    )
  }

  // ============================
  // 布局视图（idle / scanning / review / cleaning 共用 ScanPageLayout）
  // phase → 状态文案 / 主按钮 / 进度的映射集中在此（唯一 UI 决策点），
  // 布局壳与内容组件均不感知 phase，新增阶段只需在此追加分支
  // ============================
  const isIdle = phase === 'idle'
  const isScanning = phase === 'scanning'
  const isCleaning = phase === 'cleaning'

  // ---- phase → 头部状态文案 ----
  const statusContent = isScanning ? (
    <CleanStatusBar
      status="scanning"
      scanTarget={scanTarget}
      accumulatedSizeKb={accumulatedSizeKb}
      scanningText={isAuthorizing ? t('clean.status.authorizing') : cancelling ? t('clean.status.cancellingScan') : undefined}
    />
  ) : isIdle ? (
    <CleanStatusBar status="idle" error={scanError ?? undefined} />
  ) : isCleaning ? (
    <CleanStatusBar status="cleaning" cleanCurrent={cleanCurrent} />
  ) : (
    <CleanStatusBar
      status={displayStatus}
      totalSize={totalSize}
      selectedSize={totalSelectedSize}
      onBack={handleBackToIdle}
    />
  )

  // ---- phase → 主操作按钮 ----
  const action: ScanPageActionConfig = isScanning
    ? { label: cancelling ? t('clean.action.cancelling') : t('clean.action.cancel'), onClick: cancelScan, disabled: cancelling }
    : isIdle
      ? { label: t('clean.action.scan'), onClick: startScan, primary: true }
      : isCleaning
        ? { label: t('clean.action.cancelClean'), onClick: cancelClean }
        : { label: t('clean.action.clean'), onClick: handleClean, primary: true, disabled: !executionAllowed || totalSelectedSize === 0 }

  return (
    <ScanPageLayout
      loading={isScanning || isCleaning}
      statusContent={statusContent}
      action={action}
      actionClassName="clean-primary-btn"
      actionStyle={PRIMARY_CTA_STYLE}
      progress={isScanning ? scanProgress : cleanProgress}
      progressAlwaysShow={isCleaning}
      footer={phase === 'review' ? <CleanFooter executionAllowed={executionAllowed} onResetToDefault={handleResetToDefault} permanentDelete={permanentDelete} onPermanentDeleteChange={handlePermanentDeleteChange} /> : undefined}
    >
      {groups.map((g) => {
        // 渐进式扫描：该组尚无到达条目且未完成 → 占位行；否则渲染真实分类行（含已到达 items）
        if (isScanning) {
          const status = computeGroupStatusBySection(scanCompletedSections, g.id)
          if (g.itemCount === 0 && status !== 'done') {
            return <ScanningGroupRow key={g.id} group={g} status={status} />
          }
        }
        return (
          <CategoryRow
            key={g.id}
            group={g}
            isExpanded={isScanning || expandedCategories.has(g.id)}
            isIdle={isIdle}
            isCleaning={isCleaning}
            isScanning={isScanning}
            cleanedItemKeys={cleanedItemKeys}
            selectedItemIds={selectedItemIds}
            onToggleCategory={toggleCategory}
            onToggleGroup={toggleGroup}
            onToggleItem={toggleItem}
            onReveal={handleReveal}
          />
        )
      })}
    </ScanPageLayout>
  )
}
