import { useEffect, useRef, useState } from 'react'
import useTauri, { EVT_STATUS_SNAPSHOT } from '@/hooks/useTauri'
import type { MoleStatusResult } from '@/types/mole'

/** sparkline ring buffer 容量：每 2s 一帧 ≈ 最近 64 秒 */
const HIST_SIZE = 32

export interface DashboardHist {
  /** 网络下行（rx）速率历史序列（MB/s） */
  netRx: number[]
  /** 网络上行（tx）速率历史序列（MB/s） */
  netTx: number[]
}

/**
 * 仪表盘数据流：订阅 controllers/status.rs 每 2s 一帧的 status::snapshot。
 *
 * watch 生命周期由 tray.rs 管理（气泡打开 start / 隐藏 stop，消费者引用计数），
 * 本 hook 只 listen 不 start/stop——dashboard 窗口随主进程常驻（hidden 状态预加载），
 * 若这里 mount 即 start，气泡从未打开也会后台持续采集。
 *
 * 网络上下行 sparkline 历史由前端 ring buffer 维护。
 * CPU / 内存 / 风扇趋势图已移除，不再维护其 ring buffer。
 */
export function useStatusSnapshot() {
  const tauri = useTauri()
  const [snap, setSnap] = useState<MoleStatusResult | null>(null)
  const [hist, setHist] = useState<DashboardHist>({
    netRx: [],
    netTx: []
  })
  const histRef = useRef<DashboardHist>({
    netRx: [],
    netTx: []
  })

  useEffect(() => {
    const ac = new AbortController()

    // Tauri listen 不依赖窗口可见性，事件在 hidden 状态也正常送达，
    // 只需在 mount 时注册一次，无需 visibilitychange 重连。
    tauri.onIpcEvent<MoleStatusResult>(
      EVT_STATUS_SNAPSHOT,
      (payload) => {
        setSnap(payload)
        const h = histRef.current
        const next: DashboardHist = {
          netRx: [...h.netRx, payload.network_history?.rx_latest ?? 0].slice(-HIST_SIZE),
          netTx: [...h.netTx, payload.network_history?.tx_latest ?? 0].slice(-HIST_SIZE)
        }
        histRef.current = next
        setHist(next)
      },
      ac.signal
    )

    // 托盘重新可见时清空 ring buffer，避免上次会话的尖峰样本
    // 延续到新会话（与后端 STALE_THRESHOLD_SECS 协同：后端清 buffer，前端也清）。
    // 不清空会导致 Sparkline 自适应纵轴被陈旧尖峰压扁，视觉上表现为“冻结”。
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        const empty: DashboardHist = { netRx: [], netTx: [] }
        histRef.current = empty
        setHist(empty)
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    return () => {
      ac.abort()
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [tauri])

  return { snap, hist }
}
