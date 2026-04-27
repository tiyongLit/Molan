import { useState, useCallback } from 'react'

/**
 * 双栈导航模型（对齐柠檬清理的后退栈 + 前进栈）
 *
 *   backStack                 forwardStack
 *   ┌─────────────────────┐   ┌─────────────────────┐
 *   │ /Users/liuy/Library │   │ /Users/liuy/Desktop │
 *   │ /Users/liuy         │   │ /Users/liuy/Work    │
 *   │ /Users              │   └─────────────────────┘
 *   │ /                   │
 *   └─────────────────────┘
 *
 * drillIn  → push backStack，清空 forwardStack
 * goBack   → pop backStack，push forwardStack
 * goForward→ pop forwardStack，push backStack
 * breadcrumbJump → pop backStack 到目标层级
 */
export function useNavigation(initialPath: string = '/') {
  const [backStack, setBackStack] = useState<string[]>([initialPath])
  const [forwardStack, setForwardStack] = useState<string[]>([])

  const canGoBack = backStack.length > 1
  const canGoForward = forwardStack.length > 0

  const drillIn = useCallback((path: string) => {
    setBackStack((prev) => {
      if (prev[prev.length - 1] === path) return prev
      return [...prev, path]
    })
    setForwardStack([])
  }, [])

  const goBack = useCallback(() => {
    let popped: string | null = null
    setBackStack((prev) => {
      if (prev.length <= 1) return prev
      popped = prev[prev.length - 1]
      return prev.slice(0, -1)
    })
    if (popped !== null) {
      setForwardStack((prev) => [...prev, popped!])
    }
  }, [])

  const goForward = useCallback(() => {
    let top: string | null = null
    setForwardStack((prev) => {
      if (prev.length === 0) return prev
      top = prev[prev.length - 1]
      return prev.slice(0, -1)
    })
    if (top !== null) {
      setBackStack((prev) => [...prev, top!])
    }
  }, [])

  const breadcrumbJump = useCallback((targetIdx: number) => {
    let removed: string[] = []
    setBackStack((prev) => {
      if (targetIdx < 0 || targetIdx >= prev.length) return prev
      if (targetIdx === prev.length - 1) return prev
      removed = prev.slice(targetIdx + 1)
      return prev.slice(0, targetIdx + 1)
    })
    if (removed.length > 0) {
      setForwardStack((prev) => [...prev, ...removed.reverse()])
    }
  }, [])

  return {
    backStack,
    forwardStack,
    canGoBack,
    canGoForward,
    currentPath: backStack[backStack.length - 1],
    drillIn,
    goBack,
    goForward,
    breadcrumbJump
  }
}
