import { useEffect, useMemo, useState } from 'react'
import { ConfigProvider, Dropdown, theme } from 'antd'
import type { MenuProps } from 'antd'
import { ArrowDownCircle, Info, Power, Settings, Settings2 } from 'lucide-react'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'
import { useAppVersion } from '@/hooks/useAppVersion'
import { CMD_MOLE_SHOW_DOCK_ICON, CMD_MOLE_DASHBOARD_HIDE } from '@/constants/tauri-commands'
import useTauri from '@/hooks/useTauri'
import { EVT_DASHBOARD_HIDE_REQUESTED } from '@/constants/tauri-events'
import { dashTheme } from './theme'
import { useI18n } from '@/i18n'
import { useDashboardMenuActions } from './useDashboardMenuActions'

/**
 * 底部固定工具栏：左 logo · 中「打开 MoleStudio2」主入口 · 右设置下拉。
 * 下拉菜单（antd Dropdown，局部 dark algorithm 适配暗色气泡）。
 */

/** 打开并聚焦主窗口，同时经命令隐藏托盘气泡（走出场动画 + 严格配对 stop_status_watch） */
async function openMainWindow() {
  try {
    // 恢复 Dock 图标（可能被 Dock 退出隐藏了）
    await invoke(CMD_MOLE_SHOW_DOCK_ICON).catch(() => {})
    const main = await WebviewWindow.getByLabel('MoleStudio')
    if (main) {
      await main.unminimize()
      await main.show()
      await main.setFocus()
    }
    // 走 Rust 命令而非直接 window.hide()：播出场动画 + 严格配对 stop_status_watch，
    // 修复旧实现直接 hide 不触发 stop 的潜在 watch 泄漏。
    await invoke(CMD_MOLE_DASHBOARD_HIDE).catch(() => {})
  } catch (e: unknown) {
    console.error('[BottomBar] open main window failed:', e)
  }
}

/** 红点徽标：有新版本时悬浮在齿轮按钮右上角（6px，对齐 Lemon 齿轮红点；macOS 系统红 #ff3b30）。
 *  pointer-events-none：纯指示层，不遮挡齿轮点击与 hover 背景。 */
function UpdateBadge() {
  return (
    <span
      className="pointer-events-none absolute right-[2px] top-[2px] rounded-full"
      style={{ width: 6, height: 6, background: '#ff3b30' }}
    />
  )
}

export function BottomBar() {
  const { state, hasUpdate, checkForUpdate, installUpdate } = useAppVersion()
  const { t, locale } = useI18n()
  const tauri = useTauri()
  // 菜单动作统一入口：齿轮下拉与托盘原生右键菜单共用（含托盘菜单 locale 同步推送）
  const { handleMenu } = useDashboardMenuActions({ state, checkForUpdate, installUpdate })

  // 齿轮下拉改为受控：托盘气泡是常驻隐藏窗——Rust hide() 不销毁 webview、组件永不卸载
  //（useEffect cleanup 不会在"离开"时触发），且隐藏动作（点托盘图标 / 失焦）都发生在
  // webview 之外，antd 的 click-outside 收不到；非受控 open 会冻结并在下次 show() 时原样复现。
  // 复位时机 = 离开时：tray.rs 的 request_hide 在滑出动画前 emit HIDE_REQUESTED
  //（此刻窗口仍可见、webview 活跃，事件立即投递；310ms 后 hide() 时状态已归零）。
  const [menuOpen, setMenuOpen] = useState(false)
  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent(EVT_DASHBOARD_HIDE_REQUESTED, () => setMenuOpen(false), ac.signal)
    return () => ac.abort()
  }, [tauri])

  const menuItems: MenuProps['items'] = useMemo(
    () => [
      {
        key: 'update',
        // 恒显示「检查更新」：检查节奏由后台静默调度负责（useAppVersion），
        // 菜单项不随后台 checking 状态变文案/置灰；重复触发由入口守卫拦截。
        label: t('dashboard.update.check'),
        icon: <ArrowDownCircle size={13} />
      },
      { key: 'settings', label: t('dashboard.menu.settings'), icon: <Settings2 size={13} /> },
      { key: 'about', label: t('dashboard.menu.about'), icon: <Info size={13} /> },
      { type: 'divider' },
      { key: 'quit', label: t('dashboard.menu.quit'), icon: <Power size={13} />, danger: true }
    ],
    [t, locale]
  )

  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-t border-[rgba(255,255,255,0.06)] bg-[rgba(0,0,0,0.18)] px-3">
      {/* 左：品牌 logo（accent 紫渐变圆角块 + M，与页面渐变同系） */}
      <div
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-[8px] text-[12px] font-black text-white"
        style={{ background: dashTheme.btnGrad }}
      >
        M
      </div>

      {/* 中：主入口 CTA */}
      <button
        onClick={openMainWindow}
        className="min-w-0 flex-1 truncate rounded-md py-1 text-center text-[12px] font-medium transition-colors hover:bg-[rgba(255,255,255,0.08)] hover:text-white"
        style={{ color: dashTheme.textSecondary }}
      >
        {t('dashboard.openMain')}
      </button>

      {/* 右：设置下拉 */}
      <ConfigProvider
        theme={{
          algorithm: theme.darkAlgorithm,
          token: { colorBgElevated: '#22252b', borderRadius: 8 }
        }}
      >
        <Dropdown
          open={menuOpen}
          onOpenChange={setMenuOpen}
          menu={{ items: menuItems, onClick: handleMenu }}
          trigger={['click']}
        >
          <button
            className="relative flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-[rgba(255,255,255,0.08)] hover:text-white"
            style={{ color: dashTheme.textTertiary }}
          >
            <Settings size={15} />
            {hasUpdate && <UpdateBadge />}
          </button>
        </Dropdown>
      </ConfigProvider>
    </div>
  )
}
