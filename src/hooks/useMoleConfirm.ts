import { invoke } from '@tauri-apps/api/core'
import { CMD_MOLE_SYSTEM_CONFIRM } from '@/constants/tauri-commands'

// ============================================================
// useMoleConfirm — 替代 @tauri-apps/plugin-dialog 的 ask()。
//
// 签名与 ask 兼容（最小破坏替换）：
//   ask(message, options?) → Promise<boolean>
//   options.kind: plugin-dialog 取值 'info' | 'warning' | 'error'
//                 本项目自定义窗口取值 'info' | 'warning' | 'critical'
//                 这里把 'error' 映射为 'critical'，对齐 NSAlertStyle 语义。
//
// 调用：
//   const moleConfirm = useMoleConfirm()
//   const ok = await moleConfirm('确认吗？', { title: '提示', kind: 'warning' })
// ============================================================

// plugin-dialog Options 的最小子集（避免引入其类型依赖）
export interface MoleConfirmOptions {
  title?: string
  kind?: 'info' | 'warning' | 'error'
  okLabel?: string
  cancelLabel?: string
}

// 与 Rust 端 ConfirmKind 对应（lowercase）
type BackendKind = 'info' | 'warning' | 'critical'

function mapKind(k?: MoleConfirmOptions['kind']): BackendKind {
  if (k === 'warning') return 'warning'
  if (k === 'error') return 'critical'
  return 'info'
}

/** 直接调用，无需 hook 上下文（命令已通过 useTauri bypass 注册，但这里直接走 invoke 也行） */
export async function moleConfirm(message: string, options?: MoleConfirmOptions): Promise<boolean> {
  try {
    return await invoke<boolean>(CMD_MOLE_SYSTEM_CONFIRM, {
      args: {
        message,
        title: options?.title,
        kind: mapKind(options?.kind),
        okLabel: options?.okLabel,
        cancelLabel: options?.cancelLabel,
      },
    })
  } catch (err) {
    console.error('[useMoleConfirm] failed, fallback to false', err)
    return false
  }
}

/** hook 形态：便于未来注入上下文（如审计、禁用态）。当前仅返回静态方法。 */
export function useMoleConfirm() {
  return moleConfirm
}

export default useMoleConfirm
