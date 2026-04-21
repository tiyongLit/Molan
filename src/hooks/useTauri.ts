import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import { EVT_CONFIG_UPDATED } from '@/constants/tauri-events'
import { TAURI_COMMANDS } from '@/constants/tauri-commands'

// Re-export 事件常量，业务从 useTauri 或本文件统一拿
export { EVT_CONFIG_UPDATED, EVT_TABLE_REFRESH } from '@/constants/tauri-events'

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
const api = TAURI_COMMANDS.reduce(
  (res, cmd) => {
    res[cmd] = async (payload?: any) => {
      console.info(`[useTauri] ${cmd} called with`, payload)
      try {
        //统一所有 Tauri 命令的第二个参数名为 args，这样前端就可以通过 { args: payload } 的方式传递参数，保持一致性。
        const data = await invoke(cmd, payload ? { args: payload } : {})
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

/**
 * 统一 Tauri 能力入口：与 useElectron 一致，由命令列表生成 api，新增命令只改 constants。
 * 用法：
 *       useTauri().get_app_config()
 *       useTauri().update_app_config({ newCfg })
 */
export default function useTauri(): TauriMethods & IpcListener {
  return { ...api, ...ipc } as TauriMethods & IpcListener
}
