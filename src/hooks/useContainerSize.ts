import { useState, useEffect, useRef } from 'react'

/**
 * 监听容器尺寸变化的 Hook
 * 替代 Treemap 和 PathBreadcrumb 中重复的 ResizeObserver 逻辑
 */
export function useContainerSize<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const obs = new ResizeObserver((entries) => {
      for (const e of entries) {
        setSize({ w: e.contentRect.width, h: e.contentRect.height })
      }
    })
    obs.observe(el)
    return () => obs.disconnect()
  }, [])

  return { ref, width: size.w, height: size.h }
}
