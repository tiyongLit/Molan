import { Outlet, useLocation } from 'react-router-dom'
import { motion, AnimatePresence } from 'motion/react'
import { useShellNav } from './ShellNavContext'

// ── 页面切换动画 ──
// direction=1（往下点）: 新页从底部往上翻
// direction=-1（往上点）: 新页从顶部往下翻

const variants = {
  enter: (dir: number) => ({
    y: dir > 0 ? '100%' : '-100%',
    opacity: 0
  }),
  center: {
    y: 0,
    opacity: 1
  },
  exit: (dir: number) => ({
    y: dir > 0 ? '-30%' : '30%',
    opacity: 0
  })
}

export function ShellContent() {
  const location = useLocation()
  const { direction } = useShellNav()

  return (
    <AnimatePresence mode="wait" custom={direction}>
      <motion.div
        key={location.pathname}
        custom={direction}
        variants={variants}
        initial="enter"
        animate="center"
        exit="exit"
        transition={{ duration: 0.25, ease: 'easeInOut' }}
        className="h-full"
      >
        <Outlet />
      </motion.div>
    </AnimatePresence>
  )
}
