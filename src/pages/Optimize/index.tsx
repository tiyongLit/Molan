import { useState, useMemo, useCallback, useEffect, useRef } from 'react'
import { useNavigate } from 'react-router-dom'
import { ScanPageLayout, type ScanPageActionConfig, moleMessage } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { useMoleConfirm } from '@/hooks/useMoleConfirm'
import OptimizeStatusBar from './components/OptimizeStatusBar'
import OptimizeSkeletonList from './components/OptimizeSkeletonList'
import SelectAllRow from './components/SelectAllRow'
import OptimizeGroupRow from './components/OptimizeGroupRow'
import DiagnosticBanner from './components/DiagnosticBanner'
import OptimizeFooter from './components/OptimizeFooter'
import OptimizeResult, { type OptimizeResultSummary } from './components/OptimizeResult'
import OptimizeLogPanel, { type OptimizeLogLine } from './components/OptimizeLogPanel'
import { OPTIMIZE_MOCK_DRY_RUN } from './mock'
import { TASK_GROUPS, OPTIMIZE_CTA_STYLE, type TaskRuntime, type OptimizeGroupDef } from './optimize.constants'
import type { MoleOptimizeResult, MoleOptimizeTask } from '@/types/mole'
import './Optimize.scss'

// ============================================================
// 页面阶段：idle → analyzing → preview → executing → done
// ============================================================
type Phase = 'idle' | 'analyzing' | 'preview' | 'executing' | 'done'

// 静态演示：该任务在执行模拟中演示「跳过」结局（对齐 Mole unchanged/skipped 语义）
const SKIP_DEMO_ACTION = 'spotlight_orphan_rules_cleanup'
const SKIP_DEMO_NOTE = '未检测到孤立规则，无需清理'

export function Optimize() {
  const navigate = useNavigate()
  const tauri = useTauri()
  const moleConfirm = useMoleConfirm()

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
  const execTimerRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // 卸载时清理模拟定时器
  useEffect(() => {
    return () => {
      if (analyzeTimerRef.current) clearInterval(analyzeTimerRef.current)
      if (execTimerRef.current) clearInterval(execTimerRef.current)
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

  // 立即分析：静态阶段模拟 dry_run（接后端后替换为 tauri.mole_optimize({ dry_run: true })）
  const startAnalyze = useCallback(() => {
    setScanResult(null)
    setDoneSummary(null)
    setExecError(null)
    setPhase('analyzing')
    setAnalyzeProgress(0)
    setAnalyzeGroupIdx(0)

    const started = Date.now()
    const DURATION = 1600
    analyzeTimerRef.current = setInterval(() => {
      const elapsed = Date.now() - started
      setAnalyzeProgress(Math.min(95, Math.round((elapsed / DURATION) * 100)))
      setAnalyzeGroupIdx(Math.min(TASK_GROUPS.length - 1, Math.floor((elapsed / DURATION) * TASK_GROUPS.length)))
      if (elapsed >= DURATION) {
        clearInterval(analyzeTimerRef.current!)
        analyzeTimerRef.current = null
        const result = OPTIMIZE_MOCK_DRY_RUN
        setScanResult(result)
        // 默认全选 safe 任务（对齐 V1）
        setSelectedActions(new Set((result.tasks ?? []).filter((t) => t.safe).map((t) => t.id)))
        setExpandedGroups(new Set(TASK_GROUPS.map((g) => g.id)))
        setPhase('preview')
      }
    }, 100)
  }, [])

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

  // 开始优化：静态阶段模拟执行（接后端后替换为 tauri.mole_optimize({ dry_run: false, selected_actions }）+
  // EVT_OPTIMIZE_PROGRESS 事件驱动）
  const handleExecute = useCallback(async () => {
    const selected = tasks.filter((t) => selectedActions.has(t.id))
    if (selected.length === 0) return

    const confirmed = await moleConfirm(`将执行 ${selected.length} 项优化任务，部分项需要管理员权限。`, {
      title: '开始执行优化？',
      kind: 'warning',
      okLabel: '开始',
      cancelLabel: '再想想',
    })
    if (!confirmed) return

    // 前置授权（对齐 Clean/Startup 与 Burrow「入口先弹认证面板」）：
    // 在开始执行前弹原生认证面板（幂等，会话内免密），避免后端
    // ensure_admin_session 在执行中途突然弹窗。用户取消 → 不开始；
    // 失败 → toast 提示但不阻断（后端按 MOLE_OPTIMIZE_SUDO_AVAILABLE 降级跳过）。
    const auth = (await tauri
      .mole_request_admin_session({ prompt: '系统优化需要管理员权限' })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.error('管理员认证失败，需要权限的优化项将被跳过')
      }
      return
    }

    const seed = new Map<string, TaskRuntime>()
    selected.forEach((t) => seed.set(t.id, { action: t.id, name: t.name, status: 'pending' }))
    setTaskRuntime(seed)
    setExecProgress(0)
    setLogs([])
    setLogExpanded(true)
    setDoneSummary(null)
    setExecError(null)
    setPhase('executing')

    appendLog('meta', `▶ 开始执行 ${selected.length} 项优化任务`)

    const runOrder = selected.map((t) => t.id)
    let applied = 0
    let skipped = 0
    let failed = 0
    let step = 0 // 每个任务占两个 tick：running → 结局

    execTimerRef.current = setInterval(() => {
      const taskIdx = Math.floor(step / 2)
      const isRunningTick = step % 2 === 0

      if (taskIdx >= runOrder.length) {
        // 全部完成
        clearInterval(execTimerRef.current!)
        execTimerRef.current = null
        appendLog('meta', `■ 全部完成：成功 ${applied}，跳过 ${skipped}`)
        setExecProgress(100)
        setDoneSummary({ applied, failed, skipped })
        setPhase('done')
        return
      }

      const action = runOrder[taskIdx]
      const task = taskMap.get(action)!

      if (isRunningTick) {
        setTaskRuntime((prev) => {
          const next = new Map(prev)
          next.set(action, { action, name: task.name, status: 'running' })
          return next
        })
        appendLog('info', `[${taskIdx + 1}/${runOrder.length}] ${task.name} — ${task.description}`)
      } else {
        const isSkip = action === SKIP_DEMO_ACTION
        if (isSkip) skipped += 1
        else applied += 1
        setTaskRuntime((prev) => {
          const next = new Map(prev)
          next.set(action, {
            action,
            name: task.name,
            status: isSkip ? 'skipped' : 'success',
            note: isSkip ? SKIP_DEMO_NOTE : undefined,
          })
          return next
        })
        appendLog(isSkip ? 'info' : 'ok', isSkip
          ? `  ⊝ ${task.name} — ${SKIP_DEMO_NOTE}`
          : `  ✓ ${task.name} 完成`)
        setExecProgress(Math.min(99, Math.round(((taskIdx + 1) / runOrder.length) * 100)))
      }
      step += 1
    }, 450)
  }, [tasks, selectedActions, taskMap, appendLog, tauri])

  const cancelExecute = useCallback(() => {
    if (execTimerRef.current) {
      clearInterval(execTimerRef.current)
      execTimerRef.current = null
    }
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
    return (
      <OptimizeResult
        summary={doneSummary}
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
    ? { label: '立即分析', onClick: startAnalyze, primary: true }
    : isAnalyzing
      ? { label: '取消', onClick: cancelAnalyze }
      : isExecuting
        ? { label: '取消优化', onClick: cancelExecute }
        : { label: `开始优化（${selectedCount}）`, onClick: handleExecute, primary: true, disabled: selectedCount === 0 }

  return (
    <ScanPageLayout
      loading={isAnalyzing || isExecuting}
      loadingColor="#fb923c"
      statusContent={statusContent}
      action={action}
      actionClassName="optimize-primary-btn"
      actionStyle={OPTIMIZE_CTA_STYLE}
      progress={isExecuting ? execProgress : analyzeProgress}
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
              <span className="text-xs">点击「立即分析」扫描可优化的系统项目</span>
            </div>
          )}
        </>
      )}
    </ScanPageLayout>
  )
}
