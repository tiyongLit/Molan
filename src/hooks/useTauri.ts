import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { EVT_CONFIG_UPDATED } from '@/constants/tauri-events'
import { TAURI_COMMANDS } from '@/constants/tauri-commands'
import { Batcher } from '@/utils/batcher'

// Re-export 事件常量，业务从 useTauri 或本文件统一拿
export {
  EVT_ANALYZE_SCAN_PROGRESS,
  EVT_ANALYZE_TRASH_PROGRESS,
  EVT_CONFIG_UPDATED,
  EVT_OPTIMIZE_PROGRESS,
  EVT_STATUS_SNAPSHOT,
  EVT_TABLE_REFRESH,
  EVT_UPDATES_BREW_PROGRESS
} from '@/constants/tauri-events'

// 与后端 Rust AppConfig 对应的前端类型（字段名与 tauri_store.rs 中保持一致）
export interface AppConfig {
  promptTone: boolean
  proxy: string
  useProxy: boolean
  deleteSegments: boolean
  openInNewWindow: boolean
  blockAds: boolean
  theme: string
  useExtension: boolean
  isMobile: boolean
  maxRunner: number
  language: string
  notifyPlacement: string
  showTerminal: boolean
  privacy: boolean
  machineId: string
}

// 按命令名自动生成 api：与 useElectron 一致，新增命令只需在 constants/tauri-commands 加一项
type CmdName = (typeof TAURI_COMMANDS)[number]

/** 模块级 Batcher 单例：同一 key 的并发 invoke 合并为一次 IPC 调用 */
const batcher = new Batcher<any>()

const api = TAURI_COMMANDS.reduce(
  (res, cmd) => {
    // start_watch / stop_watch 必须每次发送 IPC，不能用 Batcher 去重，
    // 否则组件卸载后重新装载时，Batcher 返回旧的 pending promise 导致 Rust 侧收不到新的 start IPC，
    // watch 线程无法重启，前端永远收不到 status::snapshot 事件 → disk 显示为 0。
    // mole_system_confirm 同理：同 payload 调用必须各自拿到独立的 pending promise，
    // 否则第二次 confirm 会被合并返回上一次结果，await 永远卡住。
    const bypassBatcher =
      cmd === 'mole_status_start_watch' ||
      cmd === 'mole_status_stop_watch' ||
      cmd === 'mole_system_confirm'
    res[cmd] = async (payload?: any) => {
      console.info(`[useTauri] ${cmd} called with`, payload)
      try {
        const data = bypassBatcher
          ? await invoke(cmd, payload ?? {})
          : await batcher.batch(payload ? `${cmd}:${JSON.stringify(payload)}` : cmd, () =>
              invoke(cmd, payload ?? {})
            )
        console.info(`[useTauri] ${cmd} return`, data)
        return data
      } catch (err: any) {
        console.error(`[useTauri] ${cmd} error`, err)
        throw new Error(err?.message ?? String(err))
      }
    }
    return res
  },
  {} as Record<CmdName, (payload?: any) => Promise<any>>
)

// 事件监听：Tauri listen 返回 Promise<UnlistenFn>，卸载早于 resolve 时需用 AbortSignal 避免泄漏
export interface IpcListener {
  /** 订阅事件，handler 只收 payload（与 Electron ipc 习惯一致） */
  listenIpc: <T = unknown>(eventName: string, handler: (payload: T) => void) => Promise<UnlistenFn>
  /**
   * 在 useEffect 中订阅：传入 `ac.signal`，cleanup 里 `ac.abort()` 即可，无需保存 unlisten。
   */
  onIpcEvent: <T = unknown>(
    eventName: string,
    handler: (payload: T) => void,
    signal: AbortSignal
  ) => void
  onConfigUpdated: (handler: (payload: AppConfig) => void) => Promise<UnlistenFn>
}

const ipc: IpcListener = {
  async listenIpc<T>(eventName: string, handler: (payload: T) => void) {
    console.info('[useTauri] listenIpc', eventName)
    return listen<T>(eventName, (e) => handler(e.payload))
  },

  onIpcEvent<T>(eventName: string, handler: (payload: T) => void, signal: AbortSignal) {
    if (signal.aborted) return
    console.info('[useTauri] onIpcEvent', eventName)
    listen<T>(eventName, (e) => {
      if (!signal.aborted) handler(e.payload)
    })
      .then((unlisten) => {
        if (signal.aborted) {
          unlisten()
          return
        }
        signal.addEventListener('abort', () => unlisten(), { once: true })
      })
      .catch((err) => {
        if (!signal.aborted) {
          console.error(`[useTauri] onIpcEvent ${eventName}`, err)
        }
      })
  },

  async onConfigUpdated(handler: (payload: AppConfig) => void) {
    console.info('[useTauri] onConfigUpdated')
    return ipc.listenIpc(EVT_CONFIG_UPDATED, handler)
  }
}

/** 返回类型：所有命令名作为方法 + 事件监听 */
export type TauriMethods = Record<CmdName, (payload?: any) => Promise<any>>

/** 模块级单例，避免每次 render 新对象导致 useEffect 重复订阅 */
const tauriApiSingleton = { ...api, ...ipc } as TauriMethods & IpcListener

/**
 * 统一 Tauri 能力入口：与 useElectron 一致，由命令列表生成 api，新增命令只改 constants。
 * 用法：
 *       useTauri().get_app_config()
 *       useTauri().update_app_config({ newCfg })
 */
export default function useTauri(): TauriMethods & IpcListener {
  return tauriApiSingleton
}
