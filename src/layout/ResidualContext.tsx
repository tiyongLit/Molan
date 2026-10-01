import {
  createContext,
  useContext,
  useState,
  useCallback,
  useEffect,
  useMemo,
  type ReactNode,
} from 'react'
import { useNavigate } from 'react-router-dom'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { CMD_MOLE_RESIDUAL_TAKE_PENDING } from '@/constants/tauri-commands'
import { EVT_RESIDUAL_OPEN } from '@/constants/tauri-events'
import type { ResidualOpenPayload, ResidualTarget } from '@/types/mole'

/**
 * 卸载残留定向链路全局状态（Shell 常驻层，页面切换不丢）：
 *
 * 数据流 = 后端 pending 快照 + take 命令（对齐 trash_watch「快照是事实源，
 * 事件仅通知」哲学）：
 *   - 冷启动遗留：挂载时 take 一次（通知点击冷启动拉起场景）；
 *   - 运行期点击：`uninstall::residual-open` 事件 → take → 有值用快照，
 *     为空用事件 payload 兜底 → 写 context + 导航到卸载页。
 *
 * 消费端：Uninstall 页（非空即切 orphans tab）、OrphansTab（非空即定向扫描）。
 */
interface ResidualContextType {
  /** 当前定向目标；null = 非定向（全量孤儿扫描语义） */
  residualTarget: ResidualTarget | null
  /** 设置定向目标（卸载页提示条「扫描残留」按钮复用此入口） */
  setResidualTarget: (target: ResidualTarget) => void
  /** 清除定向目标（「查看全部残留」→ 回全量扫描） */
  clearResidualTarget: () => void
}

const ResidualContext = createContext<ResidualContextType>({
  residualTarget: null,
  setResidualTarget: () => {},
  clearResidualTarget: () => {},
})

export function ResidualProvider({ children }: { children: ReactNode }) {
  const [residualTarget, setTarget] = useState<ResidualTarget | null>(null)
  const navigate = useNavigate()

  // 消费 pending 快照并跳转：take 是事实源，事件 payload 仅作兜底
  const consumePending = useCallback(
    async (fallback?: ResidualOpenPayload) => {
      let target: ResidualTarget | null = null
      try {
        target = await invoke<ResidualTarget | null>(CMD_MOLE_RESIDUAL_TAKE_PENDING)
      } catch {
        // pending 读取失败（极端场景）→ 走事件 payload 兜底
      }
      if (!target && fallback) {
        target = {
          appName: fallback.appName,
          bundleId: fallback.bundleId,
          detectedAt: Math.floor(Date.now() / 1000),
        }
      }
      if (target) {
        setTarget(target)
        navigate('/uninstall')
      }
    },
    [navigate],
  )

  // 冷启动兜底：应用启动时消费遗留 pending（点击通知冷启动拉起场景）
  useEffect(() => {
    void consumePending()
  }, [consumePending])

  // 运行期：通知点击 delegate → 后端 emit → 这里消费
  useEffect(() => {
    const unlisten = listen<ResidualOpenPayload>(EVT_RESIDUAL_OPEN, (event) => {
      void consumePending(event.payload)
    })
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [consumePending])

  const setResidualTarget = useCallback((target: ResidualTarget) => setTarget(target), [])
  const clearResidualTarget = useCallback(() => setTarget(null), [])

  const value = useMemo(
    () => ({ residualTarget, setResidualTarget, clearResidualTarget }),
    [residualTarget, setResidualTarget, clearResidualTarget],
  )

  return <ResidualContext.Provider value={value}>{children}</ResidualContext.Provider>
}

export function useResidual() {
  return useContext(ResidualContext)
}
