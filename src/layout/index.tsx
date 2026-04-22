import { useEffect, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion } from 'motion/react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { EVT_DOCK_QUIT_REQUESTED } from '@/constants/tauri-events'
import { CMD_MOLE_CONFIRM_DOCK_QUIT } from '@/constants/tauri-commands'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import Sidebar from './Sidebar'
import { ShellContent } from './ShellContent'
import { ShellNavProvider } from './ShellNavContext'
import { ScanButtonProvider } from './ScanButtonContext'
import { useActiveId } from './routing'
import { useBackgroundGradient } from './hooks/useBackgroundGradient'
import GearBackground from './GearBackground'
import { markNavClick } from '@/utils/uiTrace'
import { preloadAllRoutes } from '@/utils/preloadRoutes'

export default function Layout() {
  const navigate = useNavigate()
  const activeId = useActiveId()
  const bgGradient = useBackgroundGradient(activeId)

  // 透明窗口需要 body / #root 背景透明，且取消 overflow 裁剪
  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.body.style.backgroundColor = 'transparent'
    const root = document.getElementById('root')
    if (root) {
      root.style.overflow = 'visible'
      root.style.borderRadius = '0'
      root.style.background = 'transparent'
    }
  }, [])

  // 路由 chunk 预加载：启动 3 秒后开始后台顺序加载各页面 chunk。
  // 等 3 秒是为了让 F0/F1 采集和首页渲染先完成，不与首屏抢资源。
  // 预加载完成后，用户点击侧边栏时 React.lazy 直接命中 ESM 缓存，
  // 消除首访 2+ 秒的 chunk 等待（dev 下效果尤其明显）。
  useEffect(() => {
    const timer = setTimeout(preloadAllRoutes, 3000)
    return () => clearTimeout(timer)
  }, [])

  const handleNavigate = useCallback(
    (id: string) => {
      // 时序埋点（卡顿分析）：路由点击起点（ShellContent 在 pathname 提交时计算全链路延迟）
      markNavClick(id)
      navigate(`/${id}`)
    },
    [navigate],
  )

  const handleMouseDown = useCallback(() => {
    getCurrentWindow().startDragging()
  }, [])

  // 退出拦截：后端（主窗口关闭 / Dock / 托盘）检测到有长任务时 emit 此事件。
  // 恢复主窗口（可能被 hide）→ 弹确认框 →
  //   确认: confirm_dock_quit 置标志 + 退出进程
  //   取消: 主窗口已在前面恢复显示，无需额外操作
  useEffect(() => {
    const unlisten = listen(EVT_DOCK_QUIT_REQUESTED, async () => {
      // 确保主窗口可见（close handler 可能已隐藏窗口）
      try {
        const win = getCurrentWindow()
        await win.show()
        await win.setFocus()
      } catch { /* 窗口可能不存在，忽略 */ }

      const ok = await moleNativeConfirm(
        '当前有任务正在运行，强制退出将中断操作。确定要退出吗？',
        {
          kind: 'warning',
          okLabel: '强制退出',
          cancelLabel: '取消',
        }
      )
      if (ok) {
        await invoke(CMD_MOLE_CONFIRM_DOCK_QUIT).catch(() => {})
      }
    })
    return () => { unlisten.then(fn => fn()) }
  }, [])

  return (
    <ScanButtonProvider>
        <ShellNavProvider currentId={activeId} onNavigate={handleNavigate}>
          <div className="shell-page w-full h-full relative">
            <motion.div
              className="window-card relative flex flex-col w-full h-full"
              style={{ background: bgGradient }}
              onMouseDown={handleMouseDown}
            >
              <GearBackground />
              <div className="flex flex-1 min-h-0 overflow-hidden rounded-xl">
                <div className="mt-24 flex flex-col px-3">
                  <Sidebar />
                </div>
                <div className="flex-1 min-h-0 overflow-hidden">
                  <ShellContent />
                </div>
              </div>
            </motion.div>
          </div>
        </ShellNavProvider>
      </ScanButtonProvider>
  )
}
