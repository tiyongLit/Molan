import { useState, useEffect, useCallback } from 'react'
import { listen } from '@tauri-apps/api/event'
import { EVT_UNINSTALL_PROGRESS, EVT_UNINSTALL_COMPLETE } from '@/constants/tauri-events'

/** 卸载进度事件 payload */
export interface UninstallProgressPayload {
  appPath: string
  appName: string
  currentIndex: number
  totalCount: number
  currentAction: string
}

/** 卸载完成事件 payload */
export interface UninstallCompletePayload {
  appPath: string
  appName: string
  success: boolean
  freedBytes: number
  reason?: string
  suggestion?: string
}

/** 按钮状态 */
export type ButtonState = 'normal' | 'loading' | 'success' | 'error'

/** 操作类型 */
export type OperationType = 'uninstall' | 'clearData'

/** 正在清理的 app 状态 */
export interface CleaningAppState {
  progress: UninstallProgressPayload | null
  result: UninstallCompletePayload | null
  buttonState: ButtonState
  operationType?: OperationType
}

/**
 * 监听卸载进度事件，返回每个 app 的清理状态映射。
 *
 * @returns {
 *   cleaningApps: Map<string, CleaningAppState> - app 路径 → 清理状态
 *   reset: () => void - 清空所有状态
 *   startCleaning: (appPath: string, appName: string) => void - 立即开始清理状态（显示进度条）
 * }
 */
export function useUninstallProgress() {
  const [cleaningApps, setCleaningApps] = useState<Map<string, CleaningAppState>>(new Map())

  useEffect(() => {
    const unlistenProgress = listen<UninstallProgressPayload>(
      EVT_UNINSTALL_PROGRESS,
      (event) => {
        const { appPath } = event.payload
        setCleaningApps((prev) => {
          const next = new Map(prev)
          next.set(appPath, {
            progress: event.payload,
            result: next.get(appPath)?.result ?? null,
            buttonState: 'loading',
          })
          return next
        })
      }
    )

    const unlistenComplete = listen<UninstallCompletePayload>(
      EVT_UNINSTALL_COMPLETE,
      (event) => {
        const { appPath } = event.payload
        setCleaningApps((prev) => {
          const next = new Map(prev)
          const existing = next.get(appPath)
          next.set(appPath, {
            progress: existing?.progress ?? null,
            result: event.payload,
            buttonState: event.payload.success ? 'success' : 'error',
          })
          // 不清除状态，让用户手动收起该行或点击重试
          return next
        })
      }
    )

    return () => {
      unlistenProgress.then((fn) => fn())
      unlistenComplete.then((fn) => fn())
    }
  }, [])

  const reset = useCallback(() => {
    setCleaningApps(new Map())
  }, [])

  /** 立即开始清理状态，显示进度条（不等后端事件） */
  const startCleaning = useCallback((appPath: string, appName: string, operationType: OperationType = 'clearData') => {
    setCleaningApps((prev) => {
      const next = new Map(prev)
      next.set(appPath, {
        progress: {
          appPath,
          appName,
          currentIndex: 0,
          totalCount: 1,
          currentAction: operationType === 'uninstall' ? '正在卸载...' : '正在准备...',
        },
        result: null,
        buttonState: 'loading',
        operationType,
      })
      return next
    })
  }, [])

  return { cleaningApps, reset, startCleaning }
}
