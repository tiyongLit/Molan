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
  const abortControllerRef = useRef<AbortController | null>(null)

  useEffect(() => {
    // 设置事件监听器的函数
    const setupListener = () => {
      // 先中断旧的监听器
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
      }
      
      // 创建新的 AbortController
      const ac = new AbortController()
      abortControllerRef.current = ac
      
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
    }
    
    // 初始设置监听器
    setupListener()
    
    // 监听窗口可见性变化，重新建立事件监听
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        // 窗口重新可见时，重新建立事件监听器
        setupListener()
      }
    }
    
    document.addEventListener('visibilitychange', handleVisibilityChange)
    
    return () => {
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
      }
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [tauri])

  return { snap, hist }
}
