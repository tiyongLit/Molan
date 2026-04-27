/**
 * useIcon / useIconMap —— 系统图标加载 Hooks
 *
 * useIcon —— 单个路径的系统图标 Hook
 *   场景：磁盘卷标、单个文件/文件夹图标等。
 *
 * useIconMap —— 批量图标同步读取 Hook
 *   前提：调用方应在上游先通过 iconService.preloadIcons() 预加载图标。
 *   行为：从 iconService 缓存同步读取 native 图标，未命中时回退到 emoji。
 */

import { useEffect, useState, useMemo } from 'react'
import { iconService } from '@/utils/iconService'
import { resolveStaticIconMap } from '@/utils/staticIconMap'
import type { IconSrc } from '@/utils/iconService'
import type { IconInput } from '@/utils/staticIconMap'

// ── useIcon：单个路径 ──

export function useIcon(path: string): IconSrc {
  const [icon, setIcon] = useState<IconSrc>(null)

  useEffect(() => {
    if (!path) {
      setIcon(null)
      return
    }

    let cancelled = false
    iconService.getIcon(path).then((result) => {
      if (!cancelled) setIcon(result)
    })

    return () => {
      cancelled = true
    }
  }, [path])

  return icon
}

// ── useIconMap：批量同步读取 ──

export function useIconMap(inputs: IconInput[]): Record<string, string> {
  const baseMap = useMemo(() => resolveStaticIconMap(inputs), [inputs])

  return useMemo(() => {
    let hasDynamic = false
    const merged: Record<string, string> = {}

    for (const { path } of inputs) {
      if (!path) continue
      const icon = iconService.getCachedSync(path)
      if (icon) {
        merged[path] = icon
        hasDynamic = true
      }
    }

    return hasDynamic ? { ...baseMap, ...merged } : baseMap
  }, [inputs, baseMap])
}
