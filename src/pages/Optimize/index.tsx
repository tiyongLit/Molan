import { useState, useMemo, useCallback, useEffect, useRef } from 'react'
import { useNavigate } from 'react-router-dom'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ScanPageLayout, type ScanPageActionConfig, moleMessage } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { useI18n } from '@/i18n'
import { EVT_OPTIMIZE_PROGRESS } from '@/constants/tauri-events'
import OptimizeStatusBar from './components/OptimizeStatusBar'
import OptimizeSkeletonList from './components/OptimizeSkeletonList'
import SelectAllRow from './components/SelectAllRow'
import OptimizeGroupRow from './components/OptimizeGroupRow'
import DiagnosticBanner from './components/DiagnosticBanner'
import OptimizeFooter from './components/OptimizeFooter'
import OptimizeResult, { type OptimizeResultSummary } from './components/OptimizeResult'
import OptimizeLogPanel, { type OptimizeLogLine } from './components/OptimizeLogPanel'
import { TASK_GROUPS, OPTIMIZE_CTA_STYLE, type TaskRuntime, type OptimizeGroupDef } from './optimize.constants'
import type { MoleOptimizeResult, MoleOptimizeTask, OptimizeProgressEvent } from '@/types/mole'
import './Optimize.scss'

// ============================================================
// 页面阶段：idle → analyzing → preview → executing → done
// ============================================================
type Phase = 'idle' | 'analyzing' | 'preview' | 'executing' | 'done'

export function Optimize() {
  const navigate = useNavigate()
  const tauri = useTauri()
  const { t } = useI18n()

  const [phase, setPhase] = useState<Phase>('idle')
  const [scanResult, setScanResult] = useState<MoleOptimizeResult | null>(null)
  const [selectedActions, setSelectedActions] = useState<Set<string>>(new Set())
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set())

  // analyzing 阶段模拟进度
  const [analyzeProgress, setAnalyzeProgress] = useState(0)
  const [analyzeGroupIdx, setAnalyzeGroupIdx] = useState(0)
  const analyzeTimerRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // executing 阶段状态
  const [taskRuntime, setTaskRuntime] = useState<Map<string, TaskRuntime>>(new Map())
  const [execProgress, setExecProgress] = useState(0)
  const [logs, setLogs] = useState<OptimizeLogLine[]>([])
  const [logExpanded, setLogExpanded] = useState(true)
  const [doneSummary, setDoneSummary] = useState<OptimizeResultSummary | null>(null)
  const [execError, setExecError] = useState<string | null>(null)
  const optimizeListenerRef = useRef<UnlistenFn | null>(null)

  // 卸载时清理定时器与事件监听
  useEffect(() => {
    return () => {
      if (analyzeTimerRef.current) clearInterval(analyzeTimerRef.current)
      optimizeListenerRef.current?.()
    }
  }, [])

  const tasks = scanResult?.tasks ?? []
  const taskMap = useMemo(() => new Map<string, MoleOptimizeTask>(tasks.map((t) => [t.id, t])), [tasks])

  // 当前执行中的任务名（executing 状态条副行展示）
  const currentTaskName = useMemo(() => {
    for (const rt of taskRuntime.values()) {
      if (rt.status === 'running') return rt.name
    }
    return undefined
  }, [taskRuntime])

  const appendLog = useCallback((level: OptimizeLogLine['level'], text: string) => {
    setLogs((prev) => [...prev, { ts: Date.now(), level, text }])
  }, [])

  // ============================================================
  // 操作
  // ============================================================

  // 立即分析：调用后端 dry_run 获取真实任务列表与诊断数据
  const startAnalyze = useCallback(async () => {
    setScanResult(null)
    setDoneSummary(null)
    setExecError(null)
    setPhase('analyzing')
    setAnalyzeProgress(0)
    setAnalyzeGroupIdx(0)

    // 模拟进度（后端 dry_run 包含诊断采样，通常耗时 2-5s）
    const started = Date.now()
    const SIM_DURATION = 3000
    analyzeTimerRef.current = setInterval(() => {
      const elapsed = Date.now() - started
      setAnalyzeProgress(Math.min(90, Math.round((elapsed / SIM_DURATION) * 90)))
      setAnalyzeGroupIdx(Math.min(TASK_GROUPS.length - 1, Math.floor((elapsed / SIM_DURATION) * TASK_GROUPS.length)))
    }, 100)

    try {
      const result = await tauri.mole_optimize({ dry_run: true }) as MoleOptimizeResult

      // 清理定时器并跳至 100%
      if (analyzeTimerRef.current) {
        clearInterval(analyzeTimerRef.current)
        analyzeTimerRef.current = null
      }
      setAnalyzeProgress(100)

      setScanResult(result)
      // 默认全选 safe 任务（对齐 V1）
      setSelectedActions(new Set((result.tasks ?? []).filter((t) => t.safe).map((t) => t.id)))
      setExpandedGroups(new Set(TASK_GROUPS.map((g) => g.id)))
      setPhase('preview')
    } catch (e) {
      if (analyzeTimerRef.current) {
        clearInterval(analyzeTimerRef.current)
        analyzeTimerRef.current = null
      }
      const msg = typeof e === 'string' ? e : (e as Error)?.message || t('optimize.error.analyzeFailed')
      moleMessage.error(msg)
      setPhase('idle')
    }
  }, [tauri, t])

  const cancelAnalyze = useCallback(() => {
    if (analyzeTimerRef.current) {
      clearInterval(analyzeTimerRef.current)
      analyzeTimerRef.current = null
    }
    setPhase('idle')
  }, [])

  const toggleAction = useCallback((action: string) => {
    setSelectedActions((prev) => {
      const next = new Set(prev)
      if (next.has(action)) next.delete(action)
      else next.add(action)
      return next
    })
  }, [])

  const toggleGroup = useCallback((g: OptimizeGroupDef) => {
    const groupTasks = g.actionIds.map((id) => taskMap.get(id)).filter((t): t is MoleOptimizeTask => Boolean(t))
    const allSelected = groupTasks.length > 0 && groupTasks.every((t) => selectedActions.has(t.id))
    setSelectedActions((prev) => {
      const next = new Set(prev)
      for (const t of groupTasks) {
        if (allSelected) next.delete(t.id)
        else next.add(t.id)
      }
      return next
    })
  }, [taskMap, selectedActions])

  const toggleAll = useCallback((allChecked: boolean) => {
    setSelectedActions(allChecked ? new Set() : new Set(tasks.map((t) => t.id)))
  }, [tasks])

  const toggleExpand = useCallback((groupId: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(groupId)) next.delete(groupId)
      else next.add(groupId)
      return next
    })
  }, [])

  // 恢复至默认勾选态（对齐 Clean：全选 safe 任务）
  const handleResetToDefault = useCallback(() => {
    setSelectedActions(new Set(tasks.filter((t) => t.safe).map((t) => t.id)))
  }, [tasks])

  // 开始优化：调用后端 mole_optimize({ dry_run: false }) + optimize::progress 事件驱动
  const handleExecute = useCallback(async () => {
    const selected = tasks.filter((t) => selectedActions.has(t.id))
    if (selected.length === 0) return

    // 前置授权（对齐 Clean）：失败 → 降级提示但不阻断，
    // 后端按 MOLE_OPTIMIZE_SUDO_AVAILABLE 降级跳过需要权限的任务。
    const auth = (await tauri
      .mole_request_admin_session({ prompt: t('optimize.authPrompt') })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.warning(t('optimize.authFailed'))
      }
      // 用户取消 → 静默不开始；失败 → 降级继续
      if (auth.status === 'canceled') return
    }

    // 初始化执行状态
    const seed = new Map<string, TaskRuntime>()
    selected.forEach((t) => seed.set(t.id, { action: t.id, name: t.name, status: 'pending' }))
    setTaskRuntime(seed)
    setExecProgress(0)
    setLogs([])
    setLogExpanded(true)
    setDoneSummary(null)
    setExecError(null)
    setPhase('executing')

    appendLog('meta', t('optimize.log.startTasks', { count: selected.length }))

    // 订阅后端 optimize::progress 事件
    try {
      const unlisten = await listen<OptimizeProgressEvent>(EVT_OPTIMIZE_PROGRESS, (event) => {
        const e = event.payload
        switch (e.phase) {
          case 'begin':
            appendLog('meta', t('optimize.log.totalTasks', { count: e.total }))
            break
          case 'task_start': {
            setTaskRuntime((prev) => {
              const next = new Map(prev)
              next.set(e.action, { action: e.action, name: e.name, status: 'running' })
              return next
            })
            appendLog('info', `[${e.index}/${e.total}] ${e.name} — ${e.description}`)
            break
          }
          case 'task_skipped': {
            setTaskRuntime((prev) => {
              const next = new Map(prev)
              next.set(e.action, { action: e.action, name: e.name, status: 'skipped', note: e.reason })
              return next
            })
            appendLog('info', `  ⊝ ${e.name} — ${e.reason}`)
            setExecProgress(Math.min(99, Math.round(((e.index) / e.total) * 100)))
            break
          }
          case 'task_done': {
            setTaskRuntime((prev) => {
              const next = new Map(prev)
              next.set(e.action, {
                action: e.action,
                name: e.name,
                status: e.ok ? 'success' : 'failed',
                note: e.error ?? undefined,
              })
              return next
            })
            appendLog(
              e.ok ? 'ok' : 'info',
              e.ok
                ? t('optimize.log.taskDone', { name: e.name, outcome: e.outcome })
                : t('optimize.log.taskFailed', { name: e.name, error: e.error || e.outcome })
            )
            setExecProgress(Math.min(99, Math.round(((e.index) / e.total) * 100)))
            break
          }
          case 'complete': {
            appendLog('meta', t('optimize.log.complete', { success: e.success, skipped: e.skipped, failed: e.failed }))
            setExecProgress(100)
            setDoneSummary({ applied: e.success, failed: e.failed, skipped: e.skipped })
            setPhase('done')
            break
          }
        }
      })
      optimizeListenerRef.current = unlisten

      // 调用后端执行
      await tauri.mole_optimize({
        dry_run: false,
        selected_actions: selected.map((t) => t.id),
      })
    } catch (e) {
      const msg = typeof e === 'string' ? e : (e as Error)?.message || t('optimize.error.executeFailed')
      moleMessage.error(msg)
      setExecError(msg)
      setPhase('preview')
    } finally {
      optimizeListenerRef.current?.()
      optimizeListenerRef.current = null
    }
  }, [tasks, selectedActions, appendLog, tauri, t])

  const cancelExecute = useCallback(() => {
    optimizeListenerRef.current?.()
    optimizeListenerRef.current = null
    setPhase('preview')
  }, [])

  const handleRestart = useCallback(() => {
    startAnalyze()
  }, [startAnalyze])

  const handleFinish = useCallback(() => {
    navigate('/home')
  }, [navigate])

  // ============================================================
  // done 视图
  // ============================================================
  if (phase === 'done') {
    // 失败明细与 summary 同源（taskRuntime 由 task_done 事件逐条写入），
    // 保证明细条数与 summary.failed 一致；reason 为后端回填的失败原因
    const failedItems = [...taskRuntime.values()]
      .filter((rt) => rt.status === 'failed')
      .map((rt) => ({ id: rt.action, name: rt.name, reason: rt.note }))
    return (
      <OptimizeResult
        summary={doneSummary}
        failedItems={failedItems}
        error={execError}
        onRestart={handleRestart}
        onFinish={handleFinish}
      />
    )
  }

  // ============================================================
  // 布局视图（idle / analyzing / preview / executing 共用 ScanPageLayout）
  // phase → 状态文案 / 主按钮 / 进度的映射集中在此（唯一 UI 决策点），
  // 布局壳与内容组件均不感知 phase，新增阶段只需在此追加分支
  // ============================================================
  const isIdle = phase === 'idle'
  const isAnalyzing = phase === 'analyzing'
  const isExecuting = phase === 'executing'

  const selectedCount = selectedActions.size
  const allChecked = tasks.length > 0 && selectedCount === tasks.length
  const partialSelected = selectedCount > 0 && !allChecked

  // ---- phase → 头部状态文案 ----
  const statusContent = isAnalyzing ? (
    <OptimizeStatusBar status="analyzing" />
  ) : isExecuting ? (
    <OptimizeStatusBar status="executing" currentTask={currentTaskName} />
  ) : isIdle ? (
    <OptimizeStatusBar status="idle" error={execError ?? undefined} />
  ) : (
    <OptimizeStatusBar
      status="preview"
      taskCount={tasks.length}
      selectedCount={selectedCount}
      systemInfo={scanResult?.system_info}
      onBack={() => setPhase('idle')}
    />
  )

  // ---- phase → 主操作按钮 ----
  const action: ScanPageActionConfig = isIdle
    ? { label: t('optimize.action.analyze'), onClick: startAnalyze, primary: true }
    : isAnalyzing
      ? { label: t('optimize.action.cancel'), onClick: cancelAnalyze }
      : isExecuting
        ? { label: t('optimize.action.cancelExecute'), onClick: cancelExecute }
        : { label: t('optimize.action.execute', { count: selectedCount }), onClick: handleExecute, primary: true, disabled: selectedCount === 0 }

  return (
    <ScanPageLayout
      loading={isAnalyzing || isExecuting}
      loadingColor="#fb923c"
      statusContent={statusContent}
      action={action}
      actionClassName="optimize-primary-btn"
      actionStyle={OPTIMIZE_CTA_STYLE}
      progress={isAnalyzing ? analyzeProgress : isExecuting ? execProgress : 0}
      progressAlwaysShow={isExecuting}
      progressHueStart={30}
      progressHueEnd={15}
      progressHueSpread={20}
      banner={phase === 'preview' ? <DiagnosticBanner diagnostics={scanResult?.diagnostics} /> : undefined}
      footer={
        phase === 'preview' ? (
          <OptimizeFooter onResetToDefault={handleResetToDefault} />
        ) : isExecuting ? (
          <OptimizeLogPanel logs={logs} expanded={logExpanded} onToggle={() => setLogExpanded((v) => !v)} />
        ) : undefined
      }
    >
      {isAnalyzing ? (
        <OptimizeSkeletonList analyzeGroupIdx={analyzeGroupIdx} />
      ) : (
        <>
          {/* 全选行（preview / executing 时显示） */}
          {!isIdle && tasks.length > 0 && (
            <SelectAllRow
              allChecked={allChecked}
              partialSelected={partialSelected}
              disabled={isExecuting}
              taskCount={tasks.length}
              selectedCount={selectedCount}
              onToggleAll={toggleAll}
            />
          )}

          {/* 分组 + 任务行 */}
          {TASK_GROUPS.map((g) => {
            const groupTasks = g.actionIds
              .map((id) => taskMap.get(id))
              .filter((t): t is MoleOptimizeTask => Boolean(t))
            if (groupTasks.length === 0) return null

            return (
              <OptimizeGroupRow
                key={g.id}
                group={g}
                tasks={groupTasks}
                isExpanded={expandedGroups.has(g.id)}
                isIdle={isIdle}
                isExecuting={isExecuting}
                selectedActions={selectedActions}
                taskRuntime={taskRuntime}
                onToggleGroup={toggleGroup}
                onToggleExpand={toggleExpand}
                onToggleAction={toggleAction}
              />
            )
          })}

          {/* idle 阶段任务尚未加载时的占位说明 */}
          {isIdle && (
            <div className="mt-8 flex flex-col items-center text-white/40">
              <span className="text-3xl mb-2">⚡</span>
              <span className="text-xs">{t('optimize.idleHint')}</span>
            </div>
          )}
        </>
      )}
    </ScanPageLayout>
  )
}
