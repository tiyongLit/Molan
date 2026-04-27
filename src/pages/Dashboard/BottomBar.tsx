import { ConfigProvider, Dropdown, theme } from 'antd'
import type { MenuProps } from 'antd'
import { ArrowDownCircle, Info, Power, Settings, Settings2 } from 'lucide-react'
import { getAllWindows } from '@tauri-apps/api/window'
import { WebviewWindow, getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { moleMessage } from '@/components/ui'
import { dashTheme as t } from './theme'

/**
 * 底部固定工具栏：左 logo · 中「打开 MoleStudio2」主入口 · 右设置下拉。
 * 下拉菜单（antd Dropdown，局部 dark algorithm 适配暗色气泡）。
 */
const menuItems: MenuProps['items'] = [
  { key: 'update', label: '检查更新', icon: <ArrowDownCircle size={13} /> },
  { key: 'settings', label: '设置', icon: <Settings2 size={13} /> },
  { key: 'about', label: '关于 MoleStudio2', icon: <Info size={13} /> },
  { type: 'divider' },
  { key: 'quit', label: '退出应用', icon: <Power size={13} />, danger: true }
]

/** 打开并聚焦主窗口，同时隐藏托盘气泡（气泡定位由 tray.rs 在下次点击时重算） */
async function openMainWindow() {
  try {
    const main = await WebviewWindow.getByLabel('MoleStudio')
    if (main) {
      await main.unminimize()
      await main.show()
      await main.setFocus()
    }
    getCurrentWebviewWindow().hide()
  } catch (e: unknown) {
    console.error('[BottomBar] open main window failed:', e)
  }
}

/** 退出应用：destroy 全部窗口触发 RunEvent::ExitRequested 结束进程。
 * 不能用 close()——主窗口/气泡都注册了 preventClose→hide，close 只会隐藏。 */
async function quitApp() {
  try {
    const windows = await getAllWindows()
    await Promise.all(windows.map((w) => w.destroy()))
  } catch (e: unknown) {
    console.error('[BottomBar] quit failed:', e)
  }
}

export function BottomBar() {
  const handleMenu: MenuProps['onClick'] = ({ key }) => {
    if (key === 'quit') {
      // 完整退出进程（含 watch 线程）；窗口关闭仅隐藏不退出，二者语义区分
      quitApp()
    } else if (key === 'update') {
      // 更新能力在主窗口 Updates 模块：先聚焦主窗口，具体导航后续接入
      openMainWindow()
    } else if (key === 'settings') {
      moleMessage.info('设置功能开发中')
    } else {
      moleMessage.info('关于 MoleStudio2')
    }
  }

  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-t border-[rgba(255,255,255,0.06)] bg-[rgba(0,0,0,0.18)] px-3">
      {/* 左：品牌 logo（accent 紫渐变圆角块 + M，与页面渐变同系） */}
      <div
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-[8px] text-[12px] font-black text-white"
        style={{ background: t.btnGrad }}
      >
        M
      </div>

      {/* 中：主入口 CTA */}
      <button
        onClick={openMainWindow}
        className="min-w-0 flex-1 truncate rounded-md py-1 text-center text-[12px] font-medium transition-colors hover:bg-[rgba(255,255,255,0.08)] hover:text-white"
        style={{ color: t.textSecondary }}
      >
        打开 MoleStudio2
      </button>

      {/* 右：设置下拉 */}
      <ConfigProvider
        theme={{
          algorithm: theme.darkAlgorithm,
          token: { colorBgElevated: '#22252b', borderRadius: 8 }
        }}
      >
        <Dropdown menu={{ items: menuItems, onClick: handleMenu }} trigger={['click']}>
          <button
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-[rgba(255,255,255,0.08)] hover:text-white"
            style={{ color: t.textTertiary }}
          >
            <Settings size={15} />
          </button>
        </Dropdown>
      </ConfigProvider>
    </div>
  )
}
