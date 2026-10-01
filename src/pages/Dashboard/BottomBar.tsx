import { useEffect, useMemo, useState } from 'react'
import { ConfigProvider, Dropdown, theme } from 'antd'
import type { MenuProps } from 'antd'
import { ArrowDownCircle, Info, Power, Settings, Settings2 } from 'lucide-react'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { moleMessage } from '@/components/ui'
import { useAppVersion } from '@/hooks/useAppVersion'
import { CMD_MOLE_IS_BUSY, CMD_MOLE_CONFIRM_DOCK_QUIT, CMD_MOLE_SHOW_DOCK_ICON, CMD_MOLE_OPEN_SETTINGS_WINDOW, CMD_MOLE_DASHBOARD_HIDE } from '@/constants/tauri-commands'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import useTauri from '@/hooks/useTauri'
import { EVT_DASHBOARD_HIDE_REQUESTED } from '@/constants/tauri-events'
import { dashTheme } from './theme'
import { useI18n, type TFunction } from '@/i18n'

/** 项目 GitHub 仓库（「关于」菜单跳转目标；修改时需同步 capabilities/default.json 的 allow-open-url 白名单） */
const PROJECT_GITHUB_URL = 'https://github.com/tiyongLit/MoleStudio'

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

/** 退出应用：直接走 Rust 侧 app.exit(0)。
 * 不能用 destroy()/close()——主窗口注册了 preventClose→hide，
 * destroy() 也会触发 CloseRequested 被拦截，窗口只会被隐藏不会退出。 */
async function quitApp(t: TFunction) {
  try {
    // 检查是否有长任务在跑
    const busy = await invoke<boolean>(CMD_MOLE_IS_BUSY).catch(() => false)
    if (busy) {
      const ok = await moleNativeConfirm(
        t('dashboard.quit.confirm'),
        {
          kind: 'warning',
          okLabel: t('dashboard.quit.force'),
          cancelLabel: t('common.cancel'),
        }
      )
      if (!ok) return
    }
    // 直接走 Rust 侧 app.exit(0)，绕过 CloseRequested 拦截
    await invoke(CMD_MOLE_CONFIRM_DOCK_QUIT).catch(() => {})
  } catch (e: unknown) {
    console.error('[BottomBar] quit failed:', e)
  }
}

/** 红点徽标：有新版本时显示在菜单项右侧（macOS 系统红 #ff3b30） */
function UpdateBadge() {
  return (
    <span
      className="ml-auto inline-block shrink-0 rounded-full"
      style={{ width: 6, height: 6, background: '#ff3b30' }}
    />
  )
}

export function BottomBar() {
  const { state, hasUpdate, checkForUpdate, installUpdate } = useAppVersion()
  const { t, locale } = useI18n()
  const tauri = useTauri()

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

  const menuItems: MenuProps['items'] = useMemo(() => [
    {
      key: 'update',
      label: (
        <span className="flex w-full items-center gap-1">
          {state.checking ? t('dashboard.update.checking') : t('dashboard.update.check')}
          {hasUpdate && <UpdateBadge />}
        </span>
      ),
      icon: <ArrowDownCircle size={13} />,
      disabled: state.checking,
    },
    { key: 'settings', label: t('dashboard.menu.settings'), icon: <Settings2 size={13} /> },
    { key: 'about', label: t('dashboard.menu.about'), icon: <Info size={13} /> },
    { type: 'divider' },
    { key: 'quit', label: t('dashboard.menu.quit'), icon: <Power size={13} />, danger: true }
  ], [state.checking, hasUpdate, t, locale])

  const handleMenu: MenuProps['onClick'] = async ({ key }) => {
    if (key === 'quit') {
      quitApp(t)
    } else if (key === 'update') {
      await handleUpdateClick()
    } else if (key === 'settings') {
      invoke(CMD_MOLE_OPEN_SETTINGS_WINDOW).catch(() => {})
    } else if (key === 'about') {
      // 「关于」→ 打开项目 GitHub 仓库（浏览器接管；托盘气泡随失焦流程自动隐藏）
      openUrl(PROJECT_GITHUB_URL).catch((err) =>
        console.error('[BottomBar] open GitHub repo failed:', err)
      )
    }
  }

  const handleUpdateClick = async () => {
    // 已有结果且有新版本 → 直接安装
    if (state.result?.available) {
      const v = state.result.latest_version ?? ''
      const isMas = state.result.source === 'app_store'
      if (isMas) {
        moleMessage.info(t('dashboard.update.masHint', { version: v }))
      } else {
        moleMessage.success(t('dashboard.update.found', { version: v }))
      }
      await installUpdate()
      return
    }

    // 触发检查
    await checkForUpdate(true)

    // checkForUpdate 异步完成后 state 更新，此处用短延迟读取最新结果做提示
    // （React 批处理会在 await 后同步 state，但为安全起见给一帧缓冲）
    setTimeout(() => {
      if (state.error) {
        moleMessage.error(t('dashboard.update.checkFailed'))
      } else if (!state.result?.available) {
        moleMessage.success(t('dashboard.update.upToDate'))
      }
      // available=true 时红点已出现，用户再次点击即触发安装
    }, 200)
  }

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
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-[rgba(255,255,255,0.08)] hover:text-white"
            style={{ color: dashTheme.textTertiary }}
          >
            <Settings size={15} />
          </button>
        </Dropdown>
      </ConfigProvider>
    </div>
  )
}
