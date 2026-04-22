import { useState, useEffect, useRef } from 'react'
import { useThrottleFn } from 'ahooks'

/**
 * 监听容器尺寸变化的 Hook
 * 替代 Treemap 和 PathBreadcrumb 中重复的 ResizeObserver 逻辑
 *
 * **节流策略**：ResizeObserver 在窗口拖拽时可达 60fps，每次都触发 setState 会让
 * 上层（Treemap 重算 d3 布局 + 全部 RectBlock 重渲染）堆积卡顿。
 * 用 ahooks/useThrottleFn 限制最多 ~12fps（80ms），保证视觉跟手同时大幅降低重渲染量。
 */
export function useContainerSize<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })

  // useThrottleFn 自动管理 trailing 调用与组件卸载，无需手写 cleanup
  const { run: setSizeThrottled } = useThrottleFn(
    (w: number, h: number) => setSize({ w, h }),
    { wait: 80, leading: true, trailing: true }
  )

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const obs = new ResizeObserver((entries) => {
      for (const e of entries) {
        setSizeThrottled(e.contentRect.width, e.contentRect.height)
      }
    })
    obs.observe(el)
    return () => obs.disconnect()
  }, [setSizeThrottled])

  return { ref, width: size.w, height: size.h }
}
