import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useTauri from '@/hooks/useTauri'
import { EVT_CLEANUP_PHASE_RESULT } from '@/constants/tauri-events'
import { ESTIMATED_SCAN_PHASES } from '../scan-status'

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

  // ---- 订阅扫描阶段事件（仅 scanning 阶段有效）----
  useEffect(() => {
    if (!active) return
    let unlisten: (() => void) | undefined
    tauri
      .listenIpc<{ title?: string; section?: string; phase?: string; sizeKb?: number }>(EVT_CLEANUP_PHASE_RESULT, (p) => {
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
      })
      .then((u) => { unlisten = u })
    return () => { unlisten?.() }
  }, [active, tauri])

  /** 新一次扫描前的状态重置 */
  const reset = useCallback(() => {
    setScanProgress(0)
    setScanTarget(undefined)
    setAccumulatedSizeKb(0)
    accumulatedPhasesRef.current.clear()
    setScanCompletedSections(new Set())
  }, [])

  /** 扫描完成：进度拉满 */
  const complete = useCallback(() => {
    setScanProgress(100)
  }, [])

  return useMemo(
    () => ({ scanTarget, scanProgress, accumulatedSizeKb, scanCompletedSections, reset, complete }),
    [scanTarget, scanProgress, accumulatedSizeKb, scanCompletedSections, reset, complete]
  )
}
