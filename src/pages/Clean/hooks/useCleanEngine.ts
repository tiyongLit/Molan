import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useTauri from '@/hooks/useTauri'
import { t } from '@/i18n'
import { EVT_CLEAN_APPLY_PROGRESS } from '@/constants/tauri-events'
import type { MoleCleanItem, CleanApplyResult, CleanApplyProgressEvent } from '@/types/mole'

export interface CleanQueueEntry {
  key: string
  item: MoleCleanItem
}

export interface CleanOutcome {
  ok: boolean
  totalCleaned: number
  failedCount: number
  error: string | null
}

/**
 * 清理引擎：进度事件订阅 + 前端动画队列（模拟柠檬 slide-up 逐个消失）+ 后端调用。
 * 只在 active（phase === 'cleaning'）时订阅进度事件。
 */
export function useCleanEngine(active: boolean) {
  const tauri = useTauri()

  const [cleanProgress, setCleanProgress] = useState(0)
  const [cleanCurrent, setCleanCurrent] = useState<string | undefined>(undefined)
  // 清理动画队列（前端模拟柠檬的 slide-up 逐个消失效果）
  const [cleanedItemKeys, setCleanedItemKeys] = useState<Set<string>>(new Set())
  const cleanQueueRef = useRef<CleanQueueEntry[]>([])
  const cleanTimerRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // ---- 订阅清理进度事件（仅 cleaning 阶段有效）----
  useEffect(() => {
    if (!active) return
    let unlisten: (() => void) | undefined
    tauri
      .listenIpc<CleanApplyProgressEvent>(EVT_CLEAN_APPLY_PROGRESS, (p) => {
        if (p?.currentPath || p?.currentCategory) {
          setCleanCurrent(p.currentPath || p.currentCategory)
        }
        const total = p?.totalCategories || 0
        const done = p?.doneCategories || 0
        setCleanProgress(total > 0 ? Math.min(99, Math.round((done / total) * 100)) : 0)
      })
      .then((u) => { unlisten = u })
    return () => { unlisten?.() }
  }, [active, tauri])

  // ---- 组件卸载时清理动画定时器 ----
  useEffect(() => {
    return () => {
      if (cleanTimerRef.current) {
        clearInterval(cleanTimerRef.current)
        cleanTimerRef.current = null
      }
    }
  }, [])

  const stopTimer = useCallback(() => {
    if (cleanTimerRef.current) {
      clearInterval(cleanTimerRef.current)
      cleanTimerRef.current = null
    }
  }, [])

  /** 清理前的状态复位（弹窗判定前调用，避免 review 阶段残留上次进度条） */
  const prepare = useCallback(() => {
    setCleanProgress(0)
    setCleanCurrent(undefined)
    setCleanedItemKeys(new Set())
  }, [])

  /** 执行清理：动画队列与后端调用并行，返回归一化结果 */
  const applyClean = useCallback(async (queue: CleanQueueEntry[], scanId: string, permanentDelete: boolean = true): Promise<CleanOutcome> => {
    cleanQueueRef.current = [...queue]

    // 启动动画定时器：每隔 400ms 从队列弹出一个，触发 slide-up 动画
    const totalCount = queue.length
    cleanTimerRef.current = setInterval(() => {
      if (cleanQueueRef.current.length === 0) {
        stopTimer()
        return
      }
      const next = cleanQueueRef.current.shift()!
      setCleanedItemKeys((prev) => {
        const nextSet = new Set(prev)
        nextSet.add(next.key)
        return nextSet
      })
      const done = totalCount - cleanQueueRef.current.length
      setCleanProgress(Math.min(95, Math.round((done / totalCount) * 100)))
    }, 400)

    // 启动后端清理（与动画并行）
    try {
      const result = await tauri.clean_apply({ args: { item_ids: queue.map((k) => k.key), scan_id: scanId, permanent_delete: permanentDelete } })
      const parsed = result as CleanApplyResult
      setCleanProgress(100)
      stopTimer()
      // 等待剩余动画完成
      await new Promise((r) => setTimeout(r, 500))
      return {
        ok: true,
        totalCleaned: parsed.summary?.total_cleaned_size || 0,
        failedCount: parsed.summary?.failed_count || 0,
        error: null,
      }
    } catch (e) {
      stopTimer()
      return {
        ok: false,
        totalCleaned: 0,
        failedCount: 1,
        error: typeof e === 'string' ? e : (e as Error)?.message || t('clean.error.cleanFailed'),
      }
    }
  }, [tauri, stopTimer])

  /** 取消清理：停动画 + 通知后端 + 清空已清理标记 */
  const cancelClean = useCallback(async () => {
    stopTimer()
    try { await tauri.clean_apply_cancel() } catch { /* ignore */ }
    setCleanedItemKeys(new Set())
  }, [tauri, stopTimer])

  return useMemo(
    () => ({ cleanProgress, cleanCurrent, cleanedItemKeys, prepare, applyClean, cancelClean }),
    [cleanProgress, cleanCurrent, cleanedItemKeys, prepare, applyClean, cancelClean]
  )
}
