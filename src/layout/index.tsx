import { useEffect, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion } from 'motion/react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Sidebar from './Sidebar'
import { ShellContent } from './ShellContent'
import { CircleButton } from './CircleButton'
import { ShellNavProvider } from './ShellNavContext'
import { ScanButtonProvider } from './ScanButtonContext'
import { ScanSessionsProvider } from './ScanSessionsContext'
import { useActiveId } from './routing'
import { useBackgroundGradient } from './hooks/useBackgroundGradient'
import { activePalette } from './themeColors'
import GearBackground from './GearBackground'

export default function Layout() {
  const navigate = useNavigate()
  const activeId = useActiveId()
  const bgGradient = useBackgroundGradient(activeId)

  const theme = activePalette[activeId] ?? activePalette.home
  const circleAccent = theme.accent
  const circleBloom = theme.bloom.join(',')

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

  const handleNavigate = useCallback(
    (id: string) => navigate(`/${id}`),
    [navigate],
  )

  const handleMouseDown = useCallback(() => {
    getCurrentWindow().startDragging()
  }, [])

  return (
    <ScanSessionsProvider>
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

              {/* <div className="flex justify-center pb-4">
                <CircleButton accent={circleAccent} bloom={circleBloom} />
              </div> */}
            </motion.div>
          </div>
        </ShellNavProvider>
      </ScanButtonProvider>
    </ScanSessionsProvider>
  )
}
