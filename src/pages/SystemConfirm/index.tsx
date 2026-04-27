import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { CMD_MOLE_SYSTEM_CONFIRM_REPLY } from '@/constants/tauri-commands'
import { SEMANTIC_COLORS } from '@/constants/theme'
import './SystemConfirm.scss'

// ============================================================
// 自定义原生确认弹窗：独立 webview 窗口（替代 ask()）。
//
// 与 Lemon 自定义 NSWindowController 同形态：
//   - 透明窗口 + 中央毛玻璃卡片
//   - kind 决定图标色与主按钮色（info/warning/critical）
//   - Esc = 取消，Enter = 确认，Cmd+W = 取消（onCloseRequested 拦截）
// 用户点击或按键后 invoke `mole_system_confirm_reply` 回传，窗口由 Rust 端隐藏复用。
// ============================================================

type Kind = 'info' | 'warning' | 'critical'

interface ConfirmParams {
  id: string
  message: string
  title?: string
  kind?: Kind
  okLabel?: string
  cancelLabel?: string
}

/** 从 URL query 读取参数（Rust 端 build_query 注入）。 */
function readParams(): ConfirmParams {
  const sp = new URLSearchParams(window.location.search)
  const decode = (k: string): string | undefined => {
    const v = sp.get(k)
    return v ? decodeURIComponent(v) : undefined
  }
  const id = decode('id') ?? ''
  const message = decode('message') ?? ''
  const title = decode('title')
  const kindRaw = decode('kind')
  const kind: Kind | undefined =
    kindRaw === 'info' || kindRaw === 'warning' || kindRaw === 'critical' ? kindRaw : undefined
  const okLabel = decode('okLabel')
  const cancelLabel = decode('cancelLabel')
  return { id, message, title, kind, okLabel, cancelLabel }
}

/** kind → 图标 + 主色：对齐项目 SEMANTIC_COLORS 与 Optimize 橙色品牌色。 */
const KIND_PRESET: Record<Kind, { glyph: string; accent: string }> = {
  info: { glyph: 'i', accent: SEMANTIC_COLORS.accentBlue },
  warning: { glyph: '!', accent: '#fb923c' },
  critical: { glyph: '!', accent: SEMANTIC_COLORS.dangerRed },
}

export function SystemConfirm() {
  const params = useMemo(readParams, [])
  const [resolved, setResolved] = useState(false)

  // 透明窗口：对齐 Dashboard 处理，让 body/root 背景透明
  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.body.style.backgroundColor = 'transparent'
    const root = document.getElementById('root')
    if (root) root.style.background = 'transparent'
  }, [])

  const kind = params.kind ?? 'info'
  const preset = KIND_PRESET[kind]
  const title = params.title ?? '确认'
  const okLabel = params.okLabel ?? '确定'
  const cancelLabel = params.cancelLabel ?? '取消'

  const reply = async (confirmed: boolean) => {
    if (resolved) return
    setResolved(true)
    try {
      await invoke(CMD_MOLE_SYSTEM_CONFIRM_REPLY, {
        id: params.id,
        confirmed,
      })
    } catch (err) {
      console.error('[SystemConfirm] reply failed', err)
    }
    // 由 Rust 端隐藏窗口（复用），这里也兜底 hide
    try {
      await getCurrentWindow().hide()
    } catch (err) {
      console.error('[SystemConfirm] hide failed', err)
    }
  }

  // Esc = 取消，Enter / Return = 确认
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        void reply(false)
      } else if (e.key === 'Enter' || e.key === 'Return') {
        e.preventDefault()
        void reply(true)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // 红绿灯 / Cmd+W 关闭 = 取消
  useEffect(() => {
    let unlisten: (() => void) | undefined
    getCurrentWindow()
      .onCloseRequested(async (e) => {
        e.preventDefault()
        await reply(false)
      })
      .then((fn) => {
        unlisten = fn
      })
      .catch((err) => console.error('[SystemConfirm] onCloseRequested failed', err))
    return () => {
      unlisten?.()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="sysconfirm-root" style={{ '--accent': preset.accent } as React.CSSProperties}>
      <div className="sysconfirm-card">
        <div className="sysconfirm-icon" style={{ background: preset.accent }}>
          <span>{preset.glyph}</span>
        </div>
        <div className="sysconfirm-body">
          <div className="sysconfirm-title">{title}</div>
          <div className="sysconfirm-message">{params.message}</div>
        </div>
        <div className="sysconfirm-actions">
          <button
            type="button"
            className="sysconfirm-btn sysconfirm-btn-cancel"
            onClick={() => void reply(false)}
            autoFocus
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className="sysconfirm-btn sysconfirm-btn-ok"
            style={{ background: preset.accent }}
            onClick={() => void reply(true)}
          >
            {okLabel}
          </button>
        </div>
      </div>
    </div>
  )
}
