import { useEffect } from 'react'
import { useAnalyze } from '../contexts/AnalyzeContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { useLatest } from 'ahooks'
import { isProtectedEntrySync } from '../utils/protected'

/**
 * 键盘快捷键 Hook — 使用 useLatest 保持 Context 引用稳定，避免 useEffect 频繁重建。
 */
export function useKeyboard(containerRef: React.RefObject<HTMLDivElement | null>) {
  const ctx = useAnalyze()
  const sel = useAnalyzeSelection()

  // 用 useLatest 代替手动 ref，保持 handler 引用稳定
  const ctxLatest = useLatest(ctx)
  const selLatest = useLatest(sel)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA') return

      const c = ctxLatest.current
      const s = selLatest.current

      // Filter input is active → only respond to Escape (already handled by input onKeyDown).
      if (c.filtering) return

      switch (e.key) {
        case '/':
          if (!s.showTop20) {
            e.preventDefault()
            c.setFiltering(true)
            c.setFilterQuery('')
          }
          break
        case 'Escape':
          e.preventDefault()
          if (c.filtering) {
            c.setFiltering(false)
            c.setFilterQuery('')
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
            if (item) c.onActivate(item)
          }
          break
        case 'Backspace':
          e.preventDefault()
          if (s.showTop20) s.toggleTop20()
          else c.goBack()
          break
        case 'Delete':
          e.preventDefault()
          if (s.hasSelection) {
            c.trashSelected()
          } else if (s.focusedIdx !== null) {
            const item = s.activeData.items[s.focusedIdx]
            if (item && !isProtectedEntrySync(item)) c.trashEntry(item)
          }
          break
      }
    }

    el.addEventListener('keydown', handler)
    return () => el.removeEventListener('keydown', handler)
  }, []) // 空依赖 — useLatest 保证始终读取最新值
}
