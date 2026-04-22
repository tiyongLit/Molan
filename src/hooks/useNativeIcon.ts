/**
 * useNativeIcon / useNativeIconMap —— 原生图标 hooks（订阅 NativeIconRegistry）。
 *
 * useNativeIcon(path)
 *   返回 SVG data URI（已缓存）或 null（未命中 / 未解析）。
 *   挂载时自动 resolveSingle，解析完成后注册表通知重渲染；
 *   适合单个路径场景（Home 磁盘卷标、面包屑、启动项 plist 图标等）。
 *
 * useNativeIconMap(paths)
 *   返回 Record<path, SVG data URI | null>，订阅注册表变更；
 *   适合批量场景（Analyze 列表、Treemap）。
 *
 * 快照缓存：`useSyncExternalStore` 要求 getSnapshot 返回稳定引用 ——
 * 这里按「注册表版本号 + paths 逐元素相等」缓存上一份快照，避免每次
 * getSnapshot 返回新对象导致的无限重渲染（React 会直接抛错）。
 * 版本号变化但值逐元素全等时（无关路径的图标解析完成也会 bump 版本），
 * 同样复用旧快照引用，不触发重渲染。
 *
 * 与旧 `useIcon` / `useIconMap` 的对比：
 *   - 旧：iconService.getCachedSync 同步读，需要调用方自己管 iconVersion bump；
 *   - 新：hook 内部订阅注册表，写入即触发重渲染，调用方不再需要 iconVersion。
 */

import { useEffect, useRef, useSyncExternalStore } from 'react'
import { nativeIconRegistry } from '@/utils/nativeIconRegistry'

/**
 * 单个路径的原生图标（SVG data URI）；未命中返回 null。
 *
 * 单路径快照是原始值（string | null），天然稳定引用，无需额外缓存。
 * 未命中时挂载即触发异步解析（带请求去重，同路径并发只发一次 IPC）。
 */
export function useNativeIcon(path: string): string | null {
  const src = useSyncExternalStore(
    (cb) => nativeIconRegistry.subscribe(cb),
    () => (path ? nativeIconRegistry.get(path) : null)
  )

  useEffect(() => {
    if (path && !nativeIconRegistry.get(path)) {
      nativeIconRegistry.resolveSingle(path).catch(() => {})
    }
  }, [path])

  return src
}

/** 逐元素比较两个路径数组（避免 paths 引用抖动导致快照误失效） */
function samePaths(a: string[], b: string[]): boolean {
  if (a === b) return true
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false
  }
  return true
}

/** 逐元素比较两份快照的值（键集合由同一 paths 生成，按新快照迭代即可） */
function sameValues(
  a: Record<string, string | null>,
  b: Record<string, string | null>
): boolean {
  for (const k in b) {
    if (a[k] !== b[k]) return false
  }
  return true
}

/**
 * 批量路径的原生图标；返回 Record<path, SVG data URI | null>。
 *
 * 使用 `useSyncExternalStore` 订阅注册表，写入即触发重渲染；
 * 值未变化时（无关路径的写入）复用旧快照，不触发重渲染；
 * 与旧 `useIconMap` 相比，调用方不需要传 iconVersion 也不需要手动 bump。
 *
 * @param paths 路径数组（建议用 useMemo 包裹保持稳定引用）
 */
export function useNativeIconMap(paths: string[]): Record<string, string | null> {
  const cacheRef = useRef<{
    version: number
    paths: string[]
    snapshot: Record<string, string | null>
  } | null>(null)

  const getSnapshot = (): Record<string, string | null> => {
    const version = nativeIconRegistry.version
    const cache = cacheRef.current
    if (cache && cache.version === version && samePaths(cache.paths, paths)) {
      return cache.snapshot
    }
    const snapshot = nativeIconRegistry.getMany(paths)
    // 值级去抖：version 变化不代表本列表的值变化（面包屑 / 无关路径的图标
    // 解析完成也会 bump version）。全等时复用旧快照引用并推进版本号，
    // 避免 provider 与全部消费者做一次无意义的整树重渲染。
    if (cache && samePaths(cache.paths, paths) && sameValues(cache.snapshot, snapshot)) {
      cache.version = version
      return cache.snapshot
    }
    cacheRef.current = { version, paths, snapshot }
    return snapshot
  }

  return useSyncExternalStore((cb) => nativeIconRegistry.subscribe(cb), getSnapshot)
}
