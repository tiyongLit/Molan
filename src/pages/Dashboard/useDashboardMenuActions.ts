import { useCallback, useEffect } from 'react'
import type { MenuProps } from 'antd'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { moleMessage } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import useTauri from '@/hooks/useTauri'
import { useI18n, type TFunction } from '@/i18n'
import type { AppVersionState } from '@/hooks/useAppVersion'
import {
  CMD_MOLE_IS_BUSY,
  CMD_MOLE_CONFIRM_DOCK_QUIT,
  CMD_MOLE_OPEN_SETTINGS_WINDOW,
  CMD_MOLE_TRAY_SET_LOCALE
} from '@/constants/tauri-commands'
import { EVT_TRAY_MENU_ACTION } from '@/constants/tauri-events'

/** 项目 GitHub 仓库（「关于」菜单跳转目标；修改时需同步 capabilities/default.json 的 allow-open-url 白名单） */
const PROJECT_GITHUB_URL = 'https://github.com/tiyongLit/MoleStudio'

/** 退出应用：直接走 Rust 侧 app.exit(0)。
 * 不能用 destroy()/close()——主窗口注册了 preventClose→hide，
 * destroy() 也会触发 CloseRequested 被拦截，窗口只会被隐藏不会退出。 */
async function quitApp(t: TFunction) {
  try {
    // 检查是否有长任务在跑
    const busy = await invoke<boolean>(CMD_MOLE_IS_BUSY).catch(() => false)
    if (busy) {
      const ok = await moleNativeConfirm(t('dashboard.quit.confirm'), {
        kind: 'warning',
        okLabel: t('dashboard.quit.force'),
        cancelLabel: t('common.cancel')
      })
      if (!ok) return
    }
    // 直接走 Rust 侧 app.exit(0)，绕过 CloseRequested 拦截
    await invoke(CMD_MOLE_CONFIRM_DOCK_QUIT).catch(() => {})
  } catch (e: unknown) {
    console.error('[useDashboardMenuActions] quit failed:', e)
  }
}

/**
 * Dashboard 托盘气泡的菜单动作统一入口。
 *
 * 齿轮下拉（antd Dropdown）与托盘原生右键菜单共用同一套动作：
 * - 齿轮下拉点击 → handleMenu（antd onClick）
 * - 托盘菜单点击 → Rust emit `tray::menu-action` → 本 hook 监听 → runMenuAction
 *
 * 同时负责把当前 locale 推送给 Rust（mole_tray_set_locale）重建托盘菜单文案：
 * dashboard 是常驻隐藏窗（监听器永活），设置窗切语言经 i18n 的 storage 事件
 * 跨窗同步到本窗 locale 变化后，此 effect 立即推送；挂载时的首次推送
 * 兼作首启占位文案（Rust 读不到 settings.json 时回退 zh-CN）的校正。
 */
export function useDashboardMenuActions(deps: {
  state: AppVersionState
  checkForUpdate: (force?: boolean) => Promise<void>
  installUpdate: () => Promise<void>
}) {
  const { state, checkForUpdate, installUpdate } = deps
  const { t, locale } = useI18n()
  const tauri = useTauri()

  const handleUpdateClick = useCallback(async () => {
    // 两条入口（齿轮下拉 / 托盘菜单）均恒显示「检查更新」、不做 disabled 态；
    // 重复触发统一在此守卫拦截：后台静默检查进行中时忽略点击（结果数秒内自然落地）。
    if (state.checking) return

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
  }, [state, checkForUpdate, installUpdate, t])

  /** 菜单动作统一分发：齿轮下拉 key 与托盘 action 同为 update/settings/about/quit */
  const runMenuAction = useCallback(
    async (key: string) => {
      if (key === 'quit') {
        await quitApp(t)
      } else if (key === 'update') {
        await handleUpdateClick()
      } else if (key === 'settings') {
        invoke(CMD_MOLE_OPEN_SETTINGS_WINDOW).catch(() => {})
      } else if (key === 'about') {
        // 「关于」→ 打开项目 GitHub 仓库（浏览器接管；托盘气泡随失焦流程自动隐藏）
        openUrl(PROJECT_GITHUB_URL).catch((err) =>
          console.error('[useDashboardMenuActions] open GitHub repo failed:', err)
        )
      }
    },
    [t, handleUpdateClick]
  )

  /** 齿轮下拉 onClick：复用统一分发 */
  const handleMenu = useCallback<NonNullable<MenuProps['onClick']>>(
    ({ key }) => {
      void runMenuAction(key)
    },
    [runMenuAction]
  )

  // 托盘原生右键菜单点击 → 同一套动作（事件由 Rust emit_to 定向投递本窗）
  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<string>(
      EVT_TRAY_MENU_ACTION,
      (action) => {
        void runMenuAction(action)
      },
      ac.signal
    )
    return () => ac.abort()
  }, [tauri, runMenuAction])

  // locale 推送：挂载 + 语言切换（含跨窗 storage 同步）时重建托盘菜单文案
  useEffect(() => {
    invoke(CMD_MOLE_TRAY_SET_LOCALE, { locale }).catch((err) =>
      console.error('[useDashboardMenuActions] tray menu locale sync failed:', err)
    )
  }, [locale])

  return { handleMenu, runMenuAction }
}
