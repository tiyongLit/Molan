import { useCallback, useEffect, useState } from 'react'
import { Loader2, ShieldAlert, ShieldCheck } from 'lucide-react'
import { moleMessage } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { dashTheme } from './theme'
import { useI18n } from '@/i18n'

/**
 * 顶部授权提示区（替换 CleanMyMac 的免费试用卡片位置）。
 * - 挂载时查 mole_privilege_capabilities 的 sudo_session_active 会话态；
 * - 未授权：橙色警示卡 + 「立即授权」CTA → mole_request_admin_session
 *   （弹系统认证框，三态返回：authorized / canceled / failed）；
 * - 已授权：折叠为绿色细条「已解锁」正向反馈，避免布局跳变。
 *
 * 状态同步策略：
 * Dashboard 窗口随主进程常驻（hidden 预加载），用户在 Clean/Uninstall/Optimize
 * 页面完成授权后后端 MOLE_AUTH_REF 已更新，但本组件无法感知。
 * 因此监听 visibilitychange 事件——托盘重新打开时重新查询后端会话态，
 * 对齐 useStatusSnapshot / useDiskStatus 的既有模式。
 */
export function AuthBanner() {
  const tauri = useTauri()
  const { t } = useI18n()
  /** null = 检测中（不渲染，避免闪烁） */
  const [authorized, setAuthorized] = useState<boolean | null>(null)
  const [requesting, setRequesting] = useState(false)

  // 提取为 useCallback 以便挂载时和 visibilitychange 复用
  const checkAuth = useCallback(() => {
    tauri
      .mole_privilege_capabilities()
      .then((res: { system_clean?: { sudo_session_active?: boolean } }) =>
        setAuthorized(Boolean(res?.system_clean?.sudo_session_active))
      )
      .catch(() => setAuthorized(false))
  }, [tauri])

  useEffect(() => {
    // 挂载时首次检查
    checkAuth()

    // 托盘重新可见时重新检查授权状态：
    // 用户可能在 Clean/Uninstall/Optimize 窗口完成了授权，
    // 后端 MOLE_AUTH_REF（进程级全局态）已更新，
    // 此处重新查询即可同步到最新状态，无需跨窗口事件广播。
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        checkAuth()
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [checkAuth])

  const handleAuthorize = async () => {
    if (requesting) return
    setRequesting(true)
    try {
      const res = await tauri.mole_request_admin_session()
      if (res?.authorized) {
        setAuthorized(true)
      } else if (res?.status === 'failed') {
        moleMessage.error(t('dashboard.auth.failed'))
      }
      // canceled：用户取消认证框，静默退出（对齐 Uninstall/Clean 入口约定）
    } catch (e) {
      console.error('[AuthBanner] mole_request_admin_session failed:', e)
      moleMessage.error(t('dashboard.auth.failed'))
    } finally {
      setRequesting(false)
    }
  }

  if (authorized === null) return null

  if (authorized) {
    return (
      <div
        className="flex w-full items-center gap-2 rounded-lg px-3 py-1"
        style={{ background: dashTheme.successBg }}
      >
        <ShieldCheck size={12} className="shrink-0" style={{ color: dashTheme.success }} />
        <span className="text-[11px] font-medium" style={{ color: dashTheme.success }}>
          {t('dashboard.auth.unlocked')}
        </span>
      </div>
    )
  }

  // 渐变主题卡片语法：白 8% 半透明底 + 无硬边框，授权 CTA 用 accent 渐变。
  // 宽度预算：托盘窗 360pt 内本行文本区仅约 209pt——dashboard.auth.* 的 en-US
  // 文案须保持短句（标题单行上限约 33 字符），否则换行会撑高卡片挤压内存列表。
  return (
    <div className="rounded-[14px] px-3 py-2" style={{ background: dashTheme.card, border: `1px solid ${dashTheme.cardBorder}` }}>
      <div className="flex items-start gap-2">
        <ShieldAlert size={14} className="mt-0.5 shrink-0" style={{ color: dashTheme.accentSoft }} />
        <div className="min-w-0 flex-1">
          <div className="text-[12px] font-semibold leading-tight" style={{ color: dashTheme.textPrimary }}>
            {t('dashboard.auth.requiredTitle')}
          </div>
          <div className="mt-0.5 text-[10px] leading-snug" style={{ color: dashTheme.textTertiary }}>
            {t('dashboard.auth.requiredDesc')}
          </div>
        </div>
        <button
          onClick={handleAuthorize}
          disabled={requesting}
          className="flex shrink-0 items-center gap-1 self-center whitespace-nowrap rounded-lg px-2.5 py-1 text-[10px] font-semibold text-white transition-all hover:brightness-90 disabled:opacity-70"
          style={{ background: dashTheme.btnGrad }}
        >
          {requesting && <Loader2 size={10} className="animate-spin" />}
          {requesting ? t('dashboard.auth.authorizing') : t('dashboard.auth.authorize')}
        </button>
      </div>
    </div>
  )
}
