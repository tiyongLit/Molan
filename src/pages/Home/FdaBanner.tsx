import { useEffect, useRef, useState } from 'react'
import { motion } from 'motion/react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  CMD_MOLE_FDA_STATUS,
  CMD_MOLE_OPEN_FDA_GUIDE_WINDOW,
} from '@/constants/tauri-commands'
import { EVT_FDA_GUIDE_CLOSED, EVT_FDA_GUIDE_SHOWN } from '@/constants/tauri-events'
import type { FdaStatus } from '@/types/mole'
import { useI18n } from '@/i18n'

/**
 * Home 软提示横幅（FDA 权限引导的"常驻软通道"，语义对齐 Burrow 的 AccessBanner）：
 * - 显隐由 Rust 计算（未授权即显示；只要权限没解决，每次启动都会重现）；
 * - **dismiss 为会话级**：点「知道了」只隐藏本次运行，下次启动还会出现——
 *   持久化 dismiss 曾让"随手关掉"的用户与功能永久失联，已弃用；
 * - 「如何开启」打开引导窗（含完整步骤说明，不消耗引导窗频率额度）；
 * - 引导窗弹出（EVT_FDA_GUIDE_SHOWN）时让位、关闭（EVT_FDA_GUIDE_CLOSED）时重新查询；
 * - 窗口激活（如从系统设置授权完切回）时重新探测：授权成功横幅自动消失。
 */
export function FdaBanner() {
  const { t } = useI18n()
  const [show, setShow] = useState(false)
  /** 会话级 dismiss：只影响本次运行，不持久化。 */
  const dismissed = useRef(false)

  useEffect(() => {
    let disposed = false
    const unlistens: UnlistenFn[] = []
    const refresh = () => {
      if (disposed || dismissed.current) return
      void invoke<FdaStatus>(CMD_MOLE_FDA_STATUS)
        .then((status) => {
          if (!disposed && !dismissed.current) setShow(status.showBanner)
        })
        .catch(() => {})
    }
    refresh()
    const subscribe = (event: string, handler: () => void) => {
      void listen(event, () => { if (!disposed) handler() })
        .then((off) => { if (disposed) off(); else unlistens.push(off) })
        .catch(() => {})
    }
    // 引导窗弹出 → 让位；引导窗关闭 → 重新查询（未授权时恢复显示）
    subscribe(EVT_FDA_GUIDE_SHOWN, () => setShow(false))
    subscribe(EVT_FDA_GUIDE_CLOSED, refresh)
    // 从系统设置授权完切回：自动重探，授权成功横幅自动消失
    const onFocus = () => refresh()
    window.addEventListener('focus', onFocus)
    return () => {
      disposed = true
      unlistens.forEach((off) => off())
      window.removeEventListener('focus', onFocus)
    }
  }, [])

  if (!show) return null

  // 主动作：打开引导窗（含完整步骤说明）再引导去系统设置——直接跳系统设置会让
  // 没看过步骤的用户在权限列表里迷茫；用户主动打开的路径不消耗引导窗频率额度。
  const openGuide = () => {
    setShow(false)
    void invoke(CMD_MOLE_OPEN_FDA_GUIDE_WINDOW).catch(() => {})
  }
  const dismiss = () => {
    dismissed.current = true
    setShow(false)
  }

  return (
    <motion.div
      role="status"
      initial={{ opacity: 0, y: -6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25 }}
      className="flex w-full items-start gap-3 rounded-xl border border-white/12 px-4 py-3"
      style={{ background: 'rgb(255 148 72 / 10%)' }}
    >
      <svg
        className="mt-0.5 shrink-0"
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="#ff9448"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
        <path d="M7 11V7a5 5 0 0 1 10 0v4" />
      </svg>
      <p className="min-w-0 flex-1 text-[12px] leading-snug text-white/90">{t('fdaGuide.bannerText')}</p>
      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={openGuide}
          className="h-6 rounded-md px-3 text-[12px] font-medium text-white transition-opacity hover:opacity-90"
          style={{ background: '#ff9448' }}
        >
          {t('fdaGuide.bannerAction')}
        </button>
        <button
          type="button"
          onClick={dismiss}
          className="h-6 rounded-md px-2 text-[12px] text-white/70 transition-colors hover:bg-white/10 hover:text-white/90"
        >
          {t('fdaGuide.bannerDismiss')}
        </button>
      </div>
    </motion.div>
  )
}
