import { useEffect, useMemo, useRef, useState, useCallback } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import CleanStatusBar, { type CleanStatus } from './components/CleanStatusBar'
import CleanLayout, { type CleanActionConfig } from './components/CleanLayout'
import ScanningSkeletonList from './components/ScanningSkeletonList'
import ScanResult from './components/ScanResult'
import CategoryRow from './components/CategoryRow'
import CleanFooter from './components/CleanFooter'
import useTauri from '@/hooks/useTauri'
import { useMoleConfirm } from '@/hooks/useMoleConfirm'
import { moleMessage } from '@/components/ui'
import './Clean.scss'

import type { MoleCleanResult, MoleCleanItem } from '@/types/mole'
import { CATEGORY_GROUPS, type CleanGroupData } from './clean.constants'
import { isCountableCleanItem, selKey } from './scan-status'
import { useScanEngine } from './hooks/useScanEngine'
import { useCleanEngine } from './hooks/useCleanEngine'
import { useSelectionPersistence } from './hooks/useSelectionPersistence'

// ============================================================
// 页面阶段：火绒式 idle → scanning → review → cleaning → done
// ============================================================
type Phase = 'idle' | 'scanning' | 'review' | 'cleaning' | 'done'

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
  const moleConfirm = useMoleConfirm()

  const [phase, setPhase] = useState<Phase>('idle')
  const [scanResult, setScanResult] = useState<MoleCleanResult | null>(null)
  const [scanError, setScanError] = useState<string | null>(null)

  const [expandedCategories, setExpandedCategories] = useState<Set<string>>(new Set())
  const [doneSummary, setDoneSummary] = useState<{ totalCleaned: number; failedCount: number } | null>(null)

  // 扫描进度（由 cleanup::phase-result 事件驱动）
  const {
    scanTarget,
    scanProgress,
    accumulatedSizeKb,
    scanCompletedSections,
    reset: resetScan,
    complete: completeScan,
  } = useScanEngine(phase === 'scanning')

  // 清理进度 + 动画队列（由 clean::apply-progress 事件驱动）
  const {
    cleanProgress,
    cleanCurrent,
    cleanedItemKeys,
    prepare: prepareClean,
    applyClean,
    cancelClean: cancelCleanEngine,
  } = useCleanEngine(phase === 'cleaning')

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

  const allCategories = scanResult?.categories || []

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
    if (phase !== 'review' || !scanResult?.categories) return
    initializeFromScan(scanResult.categories)
  }, [phase, scanResult, initializeFromScan])

  // ---- 扫描完成后默认展开所有分类 ----
  useEffect(() => {
    if (phase !== 'review') return
    setExpandedCategories(new Set(CATEGORY_GROUPS.map(g => g.id)))
  }, [phase])

  // ---- 启动扫描 ----
  const startScan = useCallback(async () => {
    setScanError(null)
    setScanResult(null)
    // 重置勾选初始化标记，保证下次扫描完成后重新从 store 加载偏好
    resetForNewScan()
    resetScan()
    // 前置授权（对齐 v1）：扫描前弹原生认证面板（幂等，已授权直接过，会话内免密）。
    // 用户取消 / 失败 → 不阻断，继续受限扫描：系统类由后端 is_admin_authorized
    // 门控自动跳过并在分类 tips 里提示，用户级清理不受影响。
    // status 三态（authorized/canceled/failed）：取消静默，失败记录日志。
    const auth = (await tauri
      .mole_request_admin_session({ prompt: '系统清理需要管理员权限' })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized && auth.status === 'failed') {
      // 取消 → 静默（用户主动行为）；失败 → toast 提示，继续受限扫描。
      moleMessage.warning('管理员认证失败，系统类清理将被跳过')
    }
    setPhase('scanning')
    try {
      const result = await tauri.clean_scan({ size_metric: 'logical' })
      const parsed = result as MoleCleanResult
      setScanResult(parsed)
      completeScan()
      setPhase('review')
    } catch (e) {
      setScanError(typeof e === 'string' ? e : (e as Error)?.message || '扫描失败')
      setPhase('idle')
    }
  }, [tauri, resetForNewScan, resetScan, completeScan])

  // ---- 从 Home 一键跳转自动启动扫描 ----
  // 仅 Home 页 DashboardDiskCard 下方的「立即扫描」会带 state.autoScan=true 跳转；侧边栏 Link 直接进入不携带 state，不会触发。
  // 用 ref 守门防止 React.StrictMode 双跑 / 后退再次进入重复触发。
  const autoScanConsumedRef = useRef(false)
  useEffect(() => {
    if (autoScanConsumedRef.current) return
    const state = location.state as { autoScan?: boolean } | null
    if (!state?.autoScan) return
    autoScanConsumedRef.current = true
    // 清空 state 避免刷新页面 / 后退再进入时重复触发
    navigate(location.pathname, { replace: true, state: null })
    startScan()
  }, [location.state, location.pathname, navigate, startScan])

  // ---- 取消扫描 ----
  const cancelScan = useCallback(async () => {
    try { await tauri.clean_scan_cancel() } catch { /* ignore */ }
  }, [tauri])

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
    if (totalSelectedSize === 0) return
    // 复位进度/动画标记（弹窗判定前，避免 review 阶段残留上次进度条）
    prepareClean()
    setDoneSummary(null)

    // 比对当前勾选 vs 后端默认勾选，有变化则弹窗询问是否记住
    if (hasChangedFromDefault()) {
      const shouldSave = await moleConfirm(
        '如果不需要，将不会记住这次更改，会恢复默认勾选项！',
        {
          title: '下次要按这次的调整来清理吗？',
          kind: 'info',
          okLabel: '需要',
          cancelLabel: '不用',
        }
      )
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
      setPhase('review') // 无 scan_id 说明扫描数据异常，回退到 review
      return
    }

    setPhase('cleaning')
    // 动画队列 + 后端调用（useCleanEngine 内部并行执行）
    const outcome = await applyClean(queue, scanId)
    if (!outcome.ok) {
      setScanError(outcome.error)
    }
    setDoneSummary({ totalCleaned: outcome.totalCleaned, failedCount: outcome.failedCount })
    setPhase('done')
  }, [totalSelectedSize, flatItems, selectedItemIds, scanResult, prepareClean, hasChangedFromDefault, persist, applyClean])

  // ---- 取消清理 ----
  const cancelClean = useCallback(async () => {
    await cancelCleanEngine()
    setPhase('review')
  }, [cancelCleanEngine])

  // ---- 返回初始态（review 状态条「返回」按钮）----
  const handleBackToIdle = useCallback(() => {
    resetForNewScan()
    setScanResult(null)
    setDoneSummary(null)
    setScanError(null)
    setPhase('idle')
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
  // 布局视图（idle / scanning / review / cleaning 共用 CleanLayout）
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
  const action: CleanActionConfig = isScanning
    ? { label: '取消', onClick: cancelScan }
    : isIdle
      ? { label: '立即扫描', onClick: startScan, primary: true }
      : isCleaning
        ? { label: '取消清理', onClick: cancelClean }
        : { label: '立即清理', onClick: handleClean, primary: true, disabled: totalSelectedSize === 0 }

  return (
    <CleanLayout
      loading={isScanning || isCleaning}
      statusContent={statusContent}
      action={action}
      progress={isScanning ? scanProgress : cleanProgress}
      progressAlwaysShow={isCleaning}
      footer={phase === 'review' ? <CleanFooter onResetToDefault={handleResetToDefault} /> : undefined}
    >
      {isScanning ? (
        <ScanningSkeletonList completedSections={scanCompletedSections} />
      ) : (
        groups.map((g) => (
          <CategoryRow
            key={g.id}
            group={g}
            isExpanded={expandedCategories.has(g.id)}
            isIdle={isIdle}
            isCleaning={isCleaning}
            cleanedItemKeys={cleanedItemKeys}
            selectedItemIds={selectedItemIds}
            onToggleCategory={toggleCategory}
            onToggleGroup={toggleGroup}
            onToggleItem={toggleItem}
            onReveal={handleReveal}
          />
        ))
      )}
    </CleanLayout>
  )
}
