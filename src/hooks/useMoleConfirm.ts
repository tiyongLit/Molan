import { invoke } from '@tauri-apps/api/core'
import { CMD_MOLE_DIALOG } from '@/constants/tauri-commands'

// ============================================================
// moleNativeConfirm — 原生 NSAlert 确认通道（唯一确认/提示机制）
//
// macOS 真原生 NSAlert：sheet 挂调用方窗口，
// 垂直三行流 = app 图标 / 粗体文案 / 原生按钮行，
// await 返回是否点击主按钮（取消/关闭/Esc = false）
//
// 调用：
//   const ok = await moleNativeConfirm('退出后需要重新对磁盘进行分析')
//   if (ok) doSomething()
// ============================================================

export interface MoleNativeDialogOptions {
  kind?: 'info' | 'warning' | 'error'
  /** 副文案（常规字重，置于主文案下方）；对应 NSAlert.informativeText */
  informativeText?: string
  okLabel?: string
  /** 默认「取消」；传 null = 单按钮提示模式（仅主按钮） */
  cancelLabel?: string | null
}

// 与 Rust 端 DialogKind 对应（lowercase）；'error' 映射 'critical' 对齐 NSAlertStyle 语义
type BackendKind = 'info' | 'warning' | 'critical'

function mapKind(k?: MoleNativeDialogOptions['kind']): BackendKind {
  if (k === 'warning') return 'warning'
  if (k === 'error') return 'critical'
  return 'info'
}

/** 原生 NSAlert 确认：await 返回是否点击主按钮（确认）；取消/关闭/Esc = false */
export async function moleNativeConfirm(
  message: string,
  options?: MoleNativeDialogOptions
): Promise<boolean> {
  try {
    return await invoke<boolean>(CMD_MOLE_DIALOG, {
      args: {
        message,
        informativeText: options?.informativeText,
        kind: mapKind(options?.kind),
        okLabel: options?.okLabel,
        // undefined = 不传 → 后端默认「取消」；null = 显式单按钮模式
        cancelLabel: options?.cancelLabel,
      },
    })
  } catch (err) {
    console.error('[moleNativeConfirm] failed, fallback to false', err)
    return false
  }
}
