import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useTauri from '@/hooks/useTauri'
import { EVT_CLEANUP_PHASE_RESULT, EVT_CLEANUP_CATEGORY_RESULT } from '@/constants/tauri-events'
import { ESTIMATED_SCAN_PHASES } from '../scan-status'
import type { MoleCleanCategory } from '@/types/mole'
import { uiTrace } from '@/utils/uiTrace'

/**
 * 扫描进度状态机（由 cleanup::phase-result 事件驱动）。
 * 只在 active（phase === 'scanning'）时订阅事件；reset 收敛六连 set 重置。
 */
export function useScanEngine(active: boolean) {
  const tauri = useTauri()

  const [scanTarget, setScanTarget] = useState<string | undefined>(undefined)
  const [scanProgress, setScanProgress] = useState(0)
  const [accumulatedSizeKb, setAccumulatedSizeKb] = useState(0)
  const accumulatedPhasesRef = useRef<Set<string>>(new Set())
  // 已完成 section 集合（用后端 section 字符串作为 key，section 维度判定"进行中"更准确）
  const [scanCompletedSections, setScanCompletedSections] = useState<Set<string>>(new Set())
  // 渐进式扫描：逐段累积后端推送的真实分类（cleanup::category-result），扫描中即用于渲染真列表
  const [streamedCategories, setStreamedCategories] = useState<MoleCleanCategory[]>([])
  // 时序埋点（卡顿分析）：本秒事件计数与最后阶段标题（tick 上报后归零）
  const evtStatsRef = useRef<{ phase: number; cat: number; lastTitle: string }>({ phase: 0, cat: 0, lastTitle: '' })

  // ---- 订阅扫描阶段事件（仅 scanning 阶段有效）----
  // 用 onIpcEvent + AbortSignal：卸载早于 listen resolve 时也能正确解绑（避免订阅泄漏）。
  useEffect(() => {
    if (!active) return
    const ac = new AbortController()
    tauri.onIpcEvent<{ title?: string; section?: string; phase?: string; sizeKb?: number }>(
      EVT_CLEANUP_PHASE_RESULT,
      (p) => {
        evtStatsRef.current.phase += 1
        if (p?.title) evtStatsRef.current.lastTitle = p.title
        if (p?.title) setScanTarget(p.title)
        setScanProgress((prev) => Math.min(95, Math.round(((prev / 100) * ESTIMATED_SCAN_PHASES + 1) / ESTIMATED_SCAN_PHASES * 100)))
        if (p?.phase && !accumulatedPhasesRef.current.has(p.phase)) {
          accumulatedPhasesRef.current.add(p.phase)
          if (p?.sizeKb) {
            setAccumulatedSizeKb((prev) => prev + p.sizeKb!)
          }
        }
        // 用后端 section 字符串记录完成的小节（section 是后端 start_section 的逻辑边界）
        if (p?.section) {
          setScanCompletedSections(prev => {
            if (prev.has(p.section!)) return prev
            const next = new Set(prev)
            next.add(p.section!)
            return next
          })
        }
      },
      ac.signal
    )
    // 渐进式：每段扫描完成即收到该分类的完整条目，按 id 去重累积（同段重复推送以最新覆盖）
    tauri.onIpcEvent<MoleCleanCategory>(
      EVT_CLEANUP_CATEGORY_RESULT,
      (cat) => {
        if (!cat?.id) return
        evtStatsRef.current.cat += 1
        uiTrace('clean.evt.cat', `id=${cat.id} items=${cat.items?.length ?? 0}`)
        setStreamedCategories((prev) => {
          const idx = prev.findIndex((c) => c.id === cat.id)
          if (idx >= 0) {
            const next = [...prev]
            next[idx] = cat
            return next
          }
          return [...prev, cat]
        })
      },
      ac.signal
    )
    return () => ac.abort()
  }, [active, tauri])

  // 时序埋点（卡顿分析）：每秒上报渲染帧率与本秒事件增量
  // （fps 低 + cat/phase 增量高 → 事件驱动的前端重渲染是卡顿主因）
  useEffect(() => {
    if (!active) return
    let frames = 0
    let raf = 0
    const onFrame = () => {
      frames += 1
      raf = requestAnimationFrame(onFrame)
    }
    raf = requestAnimationFrame(onFrame)
    const timer = setInterval(() => {
      const s = evtStatsRef.current
      uiTrace('clean.tick', `fps=${frames} phase+${s.phase} cat+${s.cat} last="${s.lastTitle}"`)
      frames = 0
      s.phase = 0
      s.cat = 0
    }, 1000)
    return () => {
      cancelAnimationFrame(raf)
      clearInterval(timer)
    }
  }, [active])

  /** 新一次扫描前的状态重置 */
  const reset = useCallback(() => {
    setScanProgress(0)
    setScanTarget(undefined)
    setAccumulatedSizeKb(0)
    accumulatedPhasesRef.current.clear()
    setScanCompletedSections(new Set())
    setStreamedCategories([])
  }, [])

  /** 扫描完成：进度拉满 */
  const complete = useCallback(() => {
    setScanProgress(100)
  }, [])

  return useMemo(
    () => ({ scanTarget, scanProgress, accumulatedSizeKb, scanCompletedSections, streamedCategories, reset, complete }),
    [scanTarget, scanProgress, accumulatedSizeKb, scanCompletedSections, streamedCategories, reset, complete]
  )
}
