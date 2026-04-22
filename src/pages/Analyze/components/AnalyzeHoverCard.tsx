import { useState, useEffect, useRef, useCallback } from 'react'
import { useAnalyzeData } from '../contexts/AnalyzeDataContext'
import { formatSize } from '@/utils/format'
import { MarqueeText } from '@/components/ui/MarqueeText'

interface HoverState {
  open: boolean
  x: number
  y: number
  path: string | null
}

/**
 * 单例悬停气泡 — 替代 RectBlock 内的 antd Popover。
 *
 * **架构动机**：
 * 原方案每个 RectBlock 包裹一个 antd Popover，50 个方格 = 50 个 Popover 实例。
 * 每次 Treemap 重渲染时，50 个 Popover 全部重建（即使方格本身没变）。
 *
 * 新方案：全局 1 个气泡，通过 document 级 mouseover 事件委托捕获悬停，
 * 定位到目标元素右侧渲染。RectBlock 只需写 `data-path` 属性。
 *
 * **功能完全保留**：图标 + 名称（跑马灯）+ 大小，对标 Lemon LMSpaceBubbleViewController。
 */
export function AnalyzeHoverCard() {
  const { treemapItems, iconMap } = useAnalyzeData()
  const [state, setState] = useState<HoverState>({ open: false, x: 0, y: 0, path: null })
  const hideTimeoutRef = useRef<number | null>(null)
  const stateRef = useRef(state)
  stateRef.current = state

  // 用 ref 持有 treemapItems，避免 useEffect 依赖频繁变化
  const itemsRef = useRef(treemapItems)
  itemsRef.current = treemapItems

  const clearHideTimeout = useCallback(() => {
    if (hideTimeoutRef.current !== null) {
      clearTimeout(hideTimeoutRef.current)
      hideTimeoutRef.current = null
    }
  }, [])

  const scheduleHide = useCallback(() => {
    clearHideTimeout()
    hideTimeoutRef.current = window.setTimeout(() => {
      setState((s) => ({ ...s, open: false, path: null }))
      hideTimeoutRef.current = null
    }, 120)
  }, [clearHideTimeout])

  useEffect(() => {
    const handleMouseOver = (e: MouseEvent) => {
      const target = e.target as HTMLElement
      // 只处理 Treemap 容器内的悬停（通过 data-treemap-container 标记）
      const container = target.closest<HTMLElement>('[data-treemap-container]')
      if (!container) {
        // 鼠标离开 Treemap 区域，隐藏气泡
        if (stateRef.current.open) scheduleHide()
        return
      }

      const el = target.closest<HTMLElement>('[data-path]')
      if (!el) {
        // 在容器内但不在任何 rect 上，隐藏气泡
        if (stateRef.current.open) scheduleHide()
        return
      }

      const path = el.dataset.path
      if (!path) return

      // 取消隐藏定时器（鼠标移入 rect）
      clearHideTimeout()

      // 如果已经是同一个 path 且气泡已打开，不更新（避免频繁 setState）
      if (stateRef.current.path === path && stateRef.current.open) return

      const rect = el.getBoundingClientRect()
      setState({
        open: true,
        x: rect.right + 8,
        y: rect.top + rect.height / 2,
        path
      })
    }

    document.addEventListener('mouseover', handleMouseOver)
    return () => {
      document.removeEventListener('mouseover', handleMouseOver)
      clearHideTimeout()
    }
  }, [scheduleHide, clearHideTimeout])

  if (!state.open || !state.path) return null

  const item = itemsRef.current.find((i) => i.path === state.path)
  if (!item) return null

  const icon = iconMap[item.path] ?? (item.isDir ? '📁' : '📄')

  return (
    <div
      style={{
        position: 'fixed',
        left: state.x,
        top: state.y,
        transform: 'translateY(-50%)',
        zIndex: 1050,
        pointerEvents: 'none'
      }}
      className="bg-[rgba(28,28,40,0.96)] backdrop-blur-md rounded-lg shadow-xl border border-white/[0.12] px-3 py-2.5"
    >
      {/* 对标 Lemon LMSpaceBubbleViewController 布局（190×48 紧凑两行） */}
      <div className="flex items-center gap-2.5 min-w-[200px] max-w-[240px] select-none">
        {icon.startsWith('data:') ? (
          <img src={icon} alt="" className="w-[30px] h-[30px] object-contain shrink-0 analyze-icon-fade" />
        ) : (
          <span className="text-2xl leading-none shrink-0 flex items-center justify-center w-[30px] h-[30px]">
            {icon}
          </span>
        )}
        <div className="flex-1 min-w-0 flex flex-col gap-0.5">
          <MarqueeText text={item.name} className="text-sm font-medium text-white leading-tight" />
          <span className="text-xs text-white/60 font-mono">{formatSize(item.size)}</span>
        </div>
      </div>
    </div>
  )
}
