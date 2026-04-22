import { useCallback, useEffect, useRef, useState } from 'react'
import useTauri from '@/hooks/useTauri'
import { EVT_CLEAN_JOB_STATE } from '@/constants/tauri-events'
import type { CleanJobSnapshot } from '@/types/mole'

const ACTIVE_STATES: ReadonlySet<CleanJobSnapshot['state']> = new Set([
  'authorizing',
  'scanning',
  'applying',
  'cancelling',
])

/**
 * Clean 任务状态机投影：活动状态由后端持有（唯一事实来源），前端只做投影。
 *
 * 协议（对齐后端 `clean_job_*` 命令）：
 * 1. 挂载先订阅事件、再查询对账（订阅消息先于查询发出，保证订阅窗口期的事件不丢）；
 * 2. 可见性/焦点恢复时补查一次（覆盖 webview 挂起/事件丢失的极端场景）；
 * 3. 所有快照按 `seq` 单调应用，乱序/过期自动丢弃；
 * 4. 扫描任务从活动态回到 idle 时，`finishedJobId` 置为该任务 id（每任务恰一次），
 *    页面据此拉取结果切视图，消费后调用 `ackFinished()` 清位——不依赖「点击后的本地状态」。
 */
export function useCleanJob() {
  const tauri = useTauri()
  const [snapshot, setSnapshot] = useState<CleanJobSnapshot | null>(null)
  // 已结束、待页面消费结果的任务 id（活动 → idle 转换时置位）
  const [finishedJobId, setFinishedJobId] = useState<string | null>(null)
  const lastSeqRef = useRef(-1)
  const mounted = useRef(false)
  const consumedResultRef = useRef<string | null>(null)
  // 已观察到活动态的扫描任务 id：活动 → idle 转换时据此置位 finishedJobId
  const pendingJobIdRef = useRef<string | null>(null)

  /** 快照唯一入口：seq 单调丢弃乱序；活动 → idle 置位 finishedJobId（每任务一次） */
  const applySnapshot = useCallback((snap: CleanJobSnapshot | null | undefined) => {
    if (!mounted.current || !snap || typeof snap.seq !== 'number') return
    if (snap.seq <= lastSeqRef.current) return
    lastSeqRef.current = snap.seq
    setSnapshot(snap)
    if (snap.state !== 'idle' && snap.job_id) {
      pendingJobIdRef.current = snap.job_id
      setFinishedJobId(null)
    } else if (snap.state === 'idle') {
      const finished = pendingJobIdRef.current ?? snap.last_finished_job_id
      pendingJobIdRef.current = null
      if (finished && finished !== consumedResultRef.current) setFinishedJobId(finished)
    }
  }, [])

  /** 消费完结束任务的结果后清位（幂等） */
  const ackFinished = useCallback(() => {
    consumedResultRef.current = finishedJobId
    setFinishedJobId(null)
  }, [finishedJobId])

  // 挂载对账 + 订阅 + 可见性/焦点补查
  useEffect(() => {
    mounted.current = true
    const ac = new AbortController()
    const reconcile = () => {
      tauri
        .clean_job_state()
        .then((snap) => { if (!ac.signal.aborted) applySnapshot(snap as CleanJobSnapshot) })
        .catch(() => { /* 对账失败不阻塞页面：后续事件/补查会再对齐 */ })
    }
    void tauri.listenIpc<CleanJobSnapshot>(EVT_CLEAN_JOB_STATE, snap => {
      if (!ac.signal.aborted) applySnapshot(snap)
    }).then(unlisten => {
      if (ac.signal.aborted) { unlisten(); return }
      ac.signal.addEventListener('abort', unlisten, { once: true })
      reconcile()
    }).catch(() => { if (!ac.signal.aborted) reconcile() })
    const onVisible = () => {
      if (document.visibilityState === 'visible') reconcile()
    }
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener('focus', onVisible)
    return () => {
      mounted.current = false
      ac.abort()
      document.removeEventListener('visibilitychange', onVisible)
      window.removeEventListener('focus', onVisible)
    }
  }, [tauri, applySnapshot])

  /** 受理扫描（幂等：已有活动任务时后端直接返回当前快照） */
  const startScan = useCallback(
    async (sizeMetric: string = 'logical') => {
      try {
        const snap = (await tauri.clean_job_start({ size_metric: sizeMetric })) as CleanJobSnapshot
        applySnapshot(snap)
        return snap
      } catch (e) {
        // 受理失败（如旧 apply 路径占用 busy）：补一次对账保持投影一致，再向上抛
        tauri.clean_job_state().then((s) => applySnapshot(s as CleanJobSnapshot)).catch(() => {})
        throw e
      }
    },
    [tauri, applySnapshot]
  )

  /** 取消当前任务（job_id 归属校验由后端完成，旧任务 id 被忽略） */
  const cancelScan = useCallback(
    async (jobId?: string) => {
      const target = jobId ?? snapshot?.job_id ?? undefined
      try {
        const snap = (await tauri.clean_job_cancel({ job_id: target })) as CleanJobSnapshot
        applySnapshot(snap)
      } catch {
        // 取消失败不阻塞 UI：可见性补查/后续事件会兜底对齐
      }
    },
    [tauri, snapshot?.job_id, applySnapshot]
  )

  /** 取回任务结果（完成后调用；job_id 归属校验由后端完成） */
  const fetchResult = useCallback(
    async (jobId: string) => {
      return tauri.clean_job_result({ job_id: jobId })
    },
    [tauri]
  )

  const state = snapshot?.state ?? 'idle'
  const isScanActive = snapshot?.kind === 'scan' && ACTIVE_STATES.has(state)

  return { snapshot, state, isScanActive, finishedJobId, ackFinished, startScan, cancelScan, fetchResult }
}
