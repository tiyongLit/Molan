import { useEffect, useState } from 'react'
import { Loader2, ShieldAlert, ShieldCheck } from 'lucide-react'
import { moleMessage } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { dashTheme as t } from './theme'

/**
 * 顶部授权提示区（替换 CleanMyMac 的免费试用卡片位置）。
 * - 挂载时查 mole_privilege_capabilities 的 sudo_session_active 会话态；
 * - 未授权：橙色警示卡 + 「立即授权」CTA → mole_request_admin_session
 *   （弹系统认证框，三态返回：authorized / canceled / failed）；
 * - 已授权：折叠为绿色细条「已解锁」正向反馈，避免布局跳变。
 */
export function AuthBanner() {
  const tauri = useTauri()
  /** null = 检测中（不渲染，避免闪烁） */
  const [authorized, setAuthorized] = useState<boolean | null>(null)
  const [requesting, setRequesting] = useState(false)

  useEffect(() => {
    tauri
      .mole_privilege_capabilities()
      .then((res: { system_clean?: { sudo_session_active?: boolean } }) =>
        setAuthorized(Boolean(res?.system_clean?.sudo_session_active))
      )
      .catch(() => setAuthorized(false))
  }, [tauri])

  const handleAuthorize = async () => {
    if (requesting) return
    setRequesting(true)
    try {
      const res = await tauri.mole_request_admin_session()
      if (res?.authorized) {
        setAuthorized(true)
      } else if (res?.status === 'failed') {
        moleMessage.error('授权失败，请重试')
      }
      // canceled：用户取消认证框，静默退出（对齐 Uninstall/Clean 入口约定）
    } catch (e) {
      console.error('[AuthBanner] mole_request_admin_session failed:', e)
      moleMessage.error('授权失败，请重试')
    } finally {
      setRequesting(false)
    }
  }

  if (authorized === null) return null

  if (authorized) {
    return (
      <div
        className="flex w-full items-center gap-2 rounded-lg px-3 py-1"
        style={{ background: t.successBg }}
      >
        <ShieldCheck size={12} className="shrink-0" style={{ color: t.success }} />
        <span className="text-[11px] font-medium" style={{ color: t.success }}>
          已授权 · 完整功能已解锁
        </span>
      </div>
    )
  }

  // 渐变主题卡片语法：白 8% 半透明底 + 无硬边框，授权 CTA 用 accent 渐变
  return (
    <div className="rounded-[14px] px-3 py-2" style={{ background: t.card, border: `1px solid ${t.cardBorder}` }}>
      <div className="flex items-start gap-2">
        <ShieldAlert size={14} className="mt-0.5 shrink-0" style={{ color: t.accentSoft }} />
        <div className="min-w-0 flex-1">
          <div className="text-[12px] font-semibold leading-tight" style={{ color: t.textPrimary }}>
            需要授权才能使用完整功能
          </div>
          <div className="mt-0.5 text-[10px] leading-snug" style={{ color: t.textTertiary }}>
            清理、优化等能力需要管理员授权后运行
          </div>
        </div>
        <button
          onClick={handleAuthorize}
          disabled={requesting}
          className="flex shrink-0 items-center gap-1 self-center rounded-lg px-2.5 py-1 text-[10px] font-semibold text-white transition-all hover:brightness-90 disabled:opacity-70"
          style={{ background: t.btnGrad }}
        >
          {requesting && <Loader2 size={10} className="animate-spin" />}
          {requesting ? '授权中' : '立即授权'}
        </button>
      </div>
    </div>
  )
}
