import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { CMD_MOLE_FDA_GUIDE_CHECK, CMD_MOLE_FDA_RELAUNCH, CMD_MOLE_OPEN_PRIVACY_SETTINGS } from '@/constants/tauri-commands'
import { EVT_FDA_GUIDE_CLOSED } from '@/constants/tauri-events'
import { cmmPalette } from '@/layout/themeColors'
import { t } from '@/i18n'

/**
 * FDA 权限引导窗（独立窗口 label=fda-guide，由 Rust runtime::fda_guide 懒创建）。
 *
 * 形制对齐腾讯柠檬 GetFullDiskPopVC：图标 + 标题 + 说明 + 3 步操作 + [去设置]/[稍后]；
 * 从系统设置切回本窗口（window focus）自动重检授权状态，成功后切换为「已开启」态
 * 且按钮变为 [完成]——探测为实时 read_dir，授权即时生效则无需重启应用。
 */

// ── 主题色（对齐 Settings 页：深藏青渐变） ──
const palette = cmmPalette.clean
const [br, bg, bb] = palette.bloom
const [dr, dg, db] = palette.deep
const pageBg = `linear-gradient(160deg, rgb(${br},${bg},${bb}) 0%, rgb(${dr},${dg},${db}) 100%)`

export function FdaGuideWindow() {
  const [authorized, setAuthorized] = useState<boolean | null>(null)
  const mounted = useRef(false)

  const check = useCallback(() => {
    void invoke<boolean>(CMD_MOLE_FDA_GUIDE_CHECK)
      .then((ok) => { if (mounted.current) setAuthorized(ok) })
      .catch(() => {})
  }, [])

  useEffect(() => {
    mounted.current = true
    // 透明窗口需要 body / #root 背景透明（对齐 Settings / TrashReminder 模式）
    document.documentElement.style.background = 'transparent'
    document.body.style.background = 'transparent'
    const root = document.getElementById('root')
    if (root) {
      root.style.background = 'transparent'
      root.style.overflow = 'hidden'
    }
    check()
    // 从系统设置勾选完切回本窗口：focus 即重检（柠檬同款授权回环）
    const onFocus = () => check()
    window.addEventListener('focus', onFocus)
    return () => {
      mounted.current = false
      window.removeEventListener('focus', onFocus)
    }
  }, [check])

  const close = () => {
    // 关闭前广播：Home 软提示横幅重新查询（未授权时恢复显示软提示）。
    void emit(EVT_FDA_GUIDE_CLOSED).catch(() => {})
    void getCurrentWindow().hide()
  }
  const openSettings = () => { void invoke(CMD_MOLE_OPEN_PRIVACY_SETTINGS).catch(() => {}) }

  const enabled = authorized === true

  return (
    <div
      className="flex h-screen w-screen select-none"
      style={{ background: 'transparent' }}
      onMouseDown={() => { void getCurrentWindow().startDragging() }}
    >
      <div
        className="flex flex-1 flex-col overflow-hidden rounded-[10px] px-7 pt-12 pb-6"
        style={{ background: pageBg }}
      >
        {/* ── 头部：状态图标 + 标题 ── */}
        <div className="flex items-center gap-3">
          <div
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full"
            style={{ background: enabled ? 'rgb(52 199 89 / 18%)' : 'rgb(255 148 72 / 16%)' }}
          >
            {enabled ? <CheckIcon /> : <LockIcon />}
          </div>
          <div className="flex min-w-0 flex-col">
            <span className="text-[15px] font-medium leading-tight text-white">
              {enabled ? t('fdaGuide.enabledTitle') : t('fdaGuide.title')}
            </span>
            {enabled && (
              <span className="mt-0.5 text-[12px] leading-snug text-[#B0BCCC]">{t('fdaGuide.enabledDesc')}</span>
            )}
          </div>
        </div>

        {!enabled && (
          <>
            <p className="mt-3 text-[12px] leading-relaxed text-[#B0BCCC]">{t('fdaGuide.desc')}</p>

            {/* ── 3 步操作说明 ── */}
            <ol className="mt-4 flex flex-col gap-2.5">
              {[t('fdaGuide.step1'), t('fdaGuide.step2'), t('fdaGuide.step3')].map((step, i) => (
                <li key={i} className="flex items-start gap-2.5">
                  <span className="mt-[1px] flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full bg-white/12 text-[11px] text-white/90">
                    {i + 1}
                  </span>
                  <span className="text-[12px] leading-snug text-white/90">{step}</span>
                </li>
              ))}
            </ol>
          </>
        )}

        {/* ── 底部按钮行 + 重启兜底 ── */}
        <div className="mt-auto flex flex-col gap-2.5 pt-4">
          <div className="flex items-center justify-end gap-3">
            {enabled ? (
              <button
                type="button"
                onClick={close}
                className="h-7 rounded-md px-5 text-[12px] font-medium text-white transition-opacity hover:opacity-90"
                style={{ background: '#34c759' }}
              >
                {t('fdaGuide.done')}
              </button>
            ) : (
              <>
                <button
                  type="button"
                  onClick={close}
                  className="h-7 rounded-md border border-white/20 px-4 text-[12px] text-white/85 transition-colors hover:bg-white/10"
                >
                  {t('fdaGuide.later')}
                </button>
                <button
                  type="button"
                  onClick={openSettings}
                  className="h-7 rounded-md px-4 text-[12px] font-medium text-white transition-opacity hover:opacity-90"
                  style={{ background: '#ff9448' }}
                >
                  {t('fdaGuide.openSettings')}
                </button>
              </>
            )}
          </div>
          {/* 已授权但仍未生效的兜底：FDA 绑定进程启动时刻，重启是可靠生效路径 */}
          {!enabled && (
            <div className="flex items-center justify-end gap-1.5">
              <span className="text-[11px] text-white/45">{t('fdaGuide.stillNotWorking')}</span>
              <button
                type="button"
                onClick={() => { void invoke(CMD_MOLE_FDA_RELAUNCH).catch(() => {}) }}
                className="text-[11px] text-[#73a6e0] underline underline-offset-2 transition-colors hover:text-[#9dc2ee]"
              >
                {t('fdaGuide.relaunch')}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

/** 锁形图标（未授权态）。 */
function LockIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#ff9448" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
    </svg>
  )
}

/** 对勾图标（已授权态）。 */
function CheckIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#34c759" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M20 6 9 17l-5-5" />
    </svg>
  )
}
