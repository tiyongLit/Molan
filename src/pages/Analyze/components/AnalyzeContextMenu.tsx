import { useState, useEffect, useCallback, useRef } from 'react'
import { Dropdown } from 'antd'
import { useAnalyzeAction } from '../contexts/AnalyzeActionContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import {
  CTX_MENU_CLASS,
  CTX_MENU_STYLE,
  CTX_MENU_POPUP_CLASS,
  type MenuEntry
} from '../typings'

interface MenuState {
  open: boolean
  x: number
  y: number
  entry: MenuEntry | null
}

/**
 * 单例右键菜单 — 整个 BrowsingView 只渲染一次。
 *
 * **架构动机**：
 * 原方案每个 EntryRow / RectBlock 各自包裹 antd Dropdown，N 个条目 = N 个 Dropdown 实例。
 * 每次 Context 变化触发重渲染时，N 个 Dropdown 全部重建（即使条目本身没变）。
 *
 * 新方案：全局 1 个 Dropdown，通过 document 级 contextmenu 事件委托捕获右键，
 * 定位到点击坐标渲染菜单。条目组件只需写 `data-path` / `data-name` / `data-protected` 属性。
 *
 * **功能完全保留**：移到废纸篓、复制路径、在访达中显示、快速查看。
 * 受保护条目（data-protected="true"）自动跳过，不显示菜单。
 */
export function AnalyzeContextMenu() {
  const { buildContextMenu } = useAnalyzeAction()
  const { setFocusedIdx } = useAnalyzeSelection()
  const [state, setState] = useState<MenuState>({ open: false, x: 0, y: 0, entry: null })

  // 用 ref 持有 activeData.items，避免 useEffect 依赖频繁变化导致事件监听器反复注册
  const { activeData } = useAnalyzeSelection()
  const activeItemsRef = useRef(activeData.items)
  activeItemsRef.current = activeData.items

  // 全局监听 contextmenu 事件（委托）
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      const el = (e.target as HTMLElement).closest<HTMLElement>('[data-path]')
      if (!el) return
      // 受保护条目不显示菜单（对齐原 Dropdown disabled 行为）
      if (el.dataset.protected === 'true') return

      e.preventDefault()
      const path = el.dataset.path
      if (!path) return
      const name = el.dataset.name ?? ''

      // 同步焦点到该条目（对齐原 Dropdown onOpenChange 行为）
      const items = activeItemsRef.current
      const idx = items.findIndex((item) => item.path === path)
      if (idx >= 0) setFocusedIdx(idx)

      setState({ open: true, x: e.clientX, y: e.clientY, entry: { path, name } })
    }

    document.addEventListener('contextmenu', handler)
    return () => document.removeEventListener('contextmenu', handler)
  }, [setFocusedIdx])

  const handleOpenChange = useCallback((open: boolean) => {
    if (!open) setState((s) => ({ ...s, open: false, entry: null }))
  }, [])

  if (!state.open || !state.entry) return null

  const { items } = buildContextMenu(state.entry)

  return (
    <Dropdown
      open
      menu={{ items, className: CTX_MENU_CLASS, style: CTX_MENU_STYLE }}
      rootClassName={CTX_MENU_POPUP_CLASS}
      trigger={[]}
      onOpenChange={handleOpenChange}
    >
      {/* 1×1 锚点：定位到右键点击坐标，pointerEvents:none 避免干扰菜单交互 */}
      <div
        style={{
          position: 'fixed',
          left: state.x,
          top: state.y,
          width: 1,
          height: 1,
          pointerEvents: 'none'
        }}
      />
    </Dropdown>
  )
}
