import { useEffect, useRef } from 'react'
import { Outlet, useLocation } from 'react-router-dom'
import { motion, AnimatePresence } from 'motion/react'
import { useShellNav } from './ShellNavContext'
import { uiTrace, takeNavClickMark, markRender, peekRenderAt } from '@/utils/uiTrace'

// ── 页面切换动画（同步翻页模式） ──
// direction=1（往下点）: 新页从底部往上翻，旧页同步往上滑出
// direction=-1（往上点）: 新页从顶部往下翻，旧页同步往下滑出
// mode="sync" 让新旧页面同时存在、同时动画，挂载开销被退出动画遮蔽

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
    y: dir > 0 ? '-100%' : '100%',
    opacity: 0
  })
}

export function ShellContent() {
  const location = useLocation()
  const { direction } = useShellNav()

  // 时序埋点（卡顿分析）：记录本 pathname 的首次 render 时刻（用于拆分下方 delay/commit）
  markRender(location.pathname)

  // 时序埋点（卡顿分析）：路由切换耗时，拆三段
  //  delay  = 点击 → React 开始渲染（调度排队 / 主线程被占）
  //  commit = 开始渲染 → 双 rAF 完成（渲染提交 + 主线程余忙）
  //  took   = 点击 → 新页渲染完成（与 clean.tick 的 fps、stall 的 gap 对照看）
  const prevPathRef = useRef(location.pathname)
  useEffect(() => {
    const prev = prevPathRef.current
    if (prev === location.pathname) return
    prevPathRef.current = location.pathname
    const mark = takeNavClickMark()
    const t0 = mark?.at ?? performance.now()
    const id = mark?.id ?? '-'
    const from = mark ? 'click' : 'init'
    const renderAt = peekRenderAt(location.pathname)
    const fmt = (v: number) => (v < 0 ? '-' : `${v.toFixed(0)}ms`)
    const delay = from === 'click' && renderAt !== null ? renderAt - t0 : -1
    let raf2 = 0
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        const now = performance.now()
        const commit = renderAt !== null ? now - renderAt : -1
        uiTrace('nav.switch', `${prev} -> ${location.pathname} id=${id} src=${from} delay=${fmt(delay)} commit=${fmt(commit)} took=${(now - t0).toFixed(1)}ms`)
      })
    })
    return () => {
      cancelAnimationFrame(raf1)
      cancelAnimationFrame(raf2)
    }
  }, [location.pathname])

  return (
    <div className="relative h-full overflow-hidden">
      <AnimatePresence mode="sync" custom={direction}>
        <motion.div
          key={location.pathname}
          custom={direction}
          variants={variants}
          initial="enter"
          animate="center"
          exit="exit"
          transition={{ duration: 0.18, ease: 'easeInOut' }}
          className="absolute inset-0"
        >
          <Outlet />
        </motion.div>
      </AnimatePresence>
    </div>
  )
}
