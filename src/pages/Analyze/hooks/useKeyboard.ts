import { useEffect } from 'react'
import { useLatest } from 'ahooks'
import { useAnalyzeNav } from '../contexts/AnalyzeNavContext'
import { useAnalyzeAction } from '../contexts/AnalyzeActionContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { isProtectedEntrySync } from '../utils/protected'

/**
 * 键盘快捷键 Hook — 使用 useLatest 保持 Context 引用稳定，避免 useEffect 频繁重建。
 *
 * 拆分后分别持有 nav / action / selection 三个 Context 的最新引用，
 * 任一 Context 变化都不会触发 useEffect 重建（空依赖数组）。
 */
export function useKeyboard(containerRef: React.RefObject<HTMLDivElement | null>) {
  const nav = useAnalyzeNav()
  const action = useAnalyzeAction()
  const sel = useAnalyzeSelection()

  // 用 useLatest 代替手动 ref，保持 handler 引用稳定
  const navLatest = useLatest(nav)
  const actionLatest = useLatest(action)
  const selLatest = useLatest(sel)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA') return

      const n = navLatest.current
      const a = actionLatest.current
      const s = selLatest.current

      // Filter input is active → only respond to Escape (already handled by input onKeyDown).
      if (a.filtering) return

      switch (e.key) {
        case '/':
          if (!s.showTop20) {
            e.preventDefault()
            a.setFiltering(true)
            a.setFilterQuery('')
          }
          break
        case 'Escape':
          e.preventDefault()
          if (a.filtering) {
            a.setFiltering(false)
            a.setFilterQuery('')
          }
          s.deselectAll()
          s.setFocusedIdx(null)
          break
        case 'ArrowDown':
          e.preventDefault()
          s.setFocusedIdx((prev) =>
            prev === null ? 0 : Math.min(prev + 1, s.activeData.items.length - 1)
          )
          break
        case 'ArrowUp':
          e.preventDefault()
          s.setFocusedIdx((prev) =>
            prev === null ? s.activeData.items.length - 1 : Math.max(prev - 1, 0)
          )
          break
        case ' ':
          e.preventDefault()
          if (s.focusedIdx !== null) {
            const item = s.activeData.items[s.focusedIdx]
            if (item && !isProtectedEntrySync(item)) s.toggleCheck(s.focusedIdx)
          }
          break
        case 'Enter':
          e.preventDefault()
          if (s.focusedIdx !== null) {
            const item = s.activeData.items[s.focusedIdx]
            if (item) a.onActivate(item)
          }
          break
        case 'Backspace':
          e.preventDefault()
          if (s.showTop20) s.toggleTop20()
          else n.goBack()
          break
        case 'Delete':
          e.preventDefault()
          if (s.hasSelection) {
            a.trashSelected()
          } else if (s.focusedIdx !== null) {
            const item = s.activeData.items[s.focusedIdx]
            if (item && !isProtectedEntrySync(item)) a.trashEntry(item)
          }
          break
      }
    }

    el.addEventListener('keydown', handler)
    return () => el.removeEventListener('keydown', handler)
  }, []) // 空依赖 — useLatest 保证始终读取最新值
}
