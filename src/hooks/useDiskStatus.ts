import { useEffect, useRef, useState } from 'react'
import useTauri, { EVT_STATUS_SNAPSHOT } from '@/hooks/useTauri'
import { useIcon } from '@/hooks/useIcon'
import { pickPrimaryDisk } from '@/utils/platform'
import type { DiskStatus, MemoryStatus, MoleStatusResult } from '@/types/mole'

export interface DiskStatusResult {
  disk: DiskStatus | undefined
  volumeIconSrc: string | null
  openStorageSettingsSupported: boolean
  memory: MemoryStatus | undefined
}

/**
 * 磁盘/内存状态（静默设计）：不启动、不停止 watch 线程。
 *
 * 持续采集生命周期由 tray.rs 管理（仅托盘气泡打开时运行，关闭后 2s 防抖停止）。
 * 本钩子挂载时通过 mole_status_once 单次全量采集取首帧快照；
 * 若托盘气泡恰好打开（watch 运行中），事件流会顺风车实时刷新本页面。
 * 每次路由进入 Home/Analyze 会重新挂载，快照随之刷新。
 */
export function useDiskStatus(tauri: ReturnType<typeof useTauri>): DiskStatusResult {
  const [snap, setSnap] = useState<MoleStatusResult | null>(null)
  // 事件到达后忽略单次快照返回值，避免旧快照覆盖较新的 watch 事件帧
  const eventArrivedRef = useRef(false)

  useEffect(() => {
    const ac = new AbortController()
    eventArrivedRef.current = false

    tauri.onIpcEvent<MoleStatusResult>(
      EVT_STATUS_SNAPSHOT,
      (payload) => {
        eventArrivedRef.current = true
        setSnap(payload)
      },
      ac.signal
    )

    // 单次快照刷新（挂载 + 窗口恢复可见时调用）
    const refreshOnce = () => {
      tauri.mole_status_once().then((s: MoleStatusResult) => {
        if (!eventArrivedRef.current) setSnap(s)
      }).catch((e: unknown) => {
        console.error('[useDiskStatus] mole_status_once failed:', e)
      })
    }
    refreshOnce()

    // 可见性感知：主窗口 hide→show（从托盘恢复）不会 remount 组件，
    // 若 watch 已停（气泡关闭），快照会冻结在隐藏前；恢复可见时补一次刷新。
    // 刷新前重置事件守卫：若 watch 已停，事件不会再到达，
    // 旧 true 值会永远阻塞 once 结果写入（数据永久冻结）。
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        eventArrivedRef.current = false
        refreshOnce()
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    return () => {
      ac.abort()
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [tauri])

  // 统一选盘口径（与托盘/Analyze 一致）：优先 '/'，无匹配回退首个
  const disk = pickPrimaryDisk(snap?.disks)
  const openStorageSettingsSupported =
    !snap?.platform || snap.platform.toLowerCase().startsWith('darwin')

  // 统一接入 IconService → 自动缓存、去重
  const volumeIconSrc = useIcon(disk?.mount ?? '/')

  return {
    disk,
    volumeIconSrc,
    openStorageSettingsSupported,
    memory: snap?.memory
  }
}
