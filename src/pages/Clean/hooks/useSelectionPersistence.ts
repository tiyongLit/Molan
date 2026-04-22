import { useCallback, useEffect, useRef, useState } from 'react'
import { load, type Store } from '@tauri-apps/plugin-store'
import { selKey } from '../scan-status'
import type { MoleCleanCategory } from '@/types/mole'

/**
 * 勾选状态 + store 偏好持久化：
 * - 后端权威默认勾选（default_selected）作为兜底
 * - store 偏好（clean_selections）覆盖默认；"恢复默认"删除偏好
 * - 每次新扫描后按"后端默认 → store 偏好"顺序初始化一次（initializedRef 守门）
 */
export function useSelectionPersistence() {
  const [selectedItemIds, setSelectedItemIds] = useState<Set<string>>(() => new Set<string>())

  // store 持久化：记录用户偏好的勾选状态
  const storeRef = useRef<Store | null>(null)
  const storeReadyRef = useRef(false)
  // 后端权威默认勾选（用于"恢复默认"和"比对变更"）
  const defaultSelectedRef = useRef<Set<string>>(new Set())
  // 本次扫描的偏好初始化是否已完成
  const initializedRef = useRef(false)

  // ---- store 初始化（组件挂载时创建）----
  useEffect(() => {
    let cancelled = false
    load('clean-preferences.bin').then((store: Store) => {
      if (cancelled) return
      storeRef.current = store
      storeReadyRef.current = true
    })
    return () => { cancelled = true }
  }, [])

  /** 新扫描前重置：清空勾选并允许下次扫描完成后重新初始化 */
  const resetForNewScan = useCallback(() => {
    initializedRef.current = false
    setSelectedItemIds(new Set())
  }, [])

  /** 扫描完成（review）时初始化勾选：后端默认 → store 偏好覆盖 */
  const initializeFromScan = useCallback((categories: MoleCleanCategory[]) => {
    if (initializedRef.current) return

    // 1. 计算后端权威默认勾选
    const defaultSelected = new Set<string>()
    for (const cat of categories) {
      for (const item of cat.items) {
        if (item.default_selected) {
          defaultSelected.add(selKey(item.categoryId || cat.id, item.id))
        }
      }
    }
    defaultSelectedRef.current = defaultSelected

    // 2. 尝试从 store 加载用户上次保存的偏好
    const store = storeRef.current
    if (store && storeReadyRef.current) {
      store.get<Record<string, boolean>>('clean_selections').then((saved) => {
        if (saved && typeof saved === 'object' && Object.keys(saved).length > 0) {
          // 有保存的偏好 → 以保存的为准
          const savedSelected = new Set<string>()
          for (const cat of categories) {
            for (const item of cat.items) {
              const key = selKey(item.categoryId || cat.id, item.id)
              if (saved[key] === true) {
                savedSelected.add(key)
              }
            }
          }
          setSelectedItemIds(savedSelected)
        } else {
          // 无保存 → 用后端默认
          setSelectedItemIds(defaultSelected)
        }
        initializedRef.current = true
      }).catch(() => {
        setSelectedItemIds(defaultSelected)
        initializedRef.current = true
      })
    } else {
      setSelectedItemIds(defaultSelected)
      initializedRef.current = true
    }
  }, [])

  /** 单子项勾选/取消 */
  const toggleItem = useCallback((categoryId: string, itemId: string) => {
    const key = selKey(categoryId, itemId)
    setSelectedItemIds((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }, [])

  /** 整组勾选/取消：allSelected=true 全取消，否则全选 */
  const toggleGroup = useCallback((items: { categoryId: string; id: string }[], allSelected: boolean) => {
    setSelectedItemIds((prev) => {
      const next = new Set(prev)
      for (const item of items) {
        const key = selKey(item.categoryId, item.id)
        if (allSelected) next.delete(key)
        else next.add(key)
      }
      return next
    })
  }, [])

  /** 恢复后端权威默认勾选并清除 store 偏好 */
  const resetToDefault = useCallback(async () => {
    setSelectedItemIds(new Set(defaultSelectedRef.current))
    const store = storeRef.current
    if (store) {
      try {
        await store.delete('clean_selections')
        await store.save()
      } catch { /* ignore */ }
    }
  }, [])

  /** 当前勾选与后端默认是否不同（"下次要按这次的调整来清理吗"弹窗判定） */
  const hasChangedFromDefault = useCallback(() => {
    const defaults = defaultSelectedRef.current
    const currentKeys = selectedItemIds
    return (
      currentKeys.size !== defaults.size ||
      [...currentKeys].some(k => !defaults.has(k)) ||
      [...defaults].some(k => !currentKeys.has(k))
    )
  }, [selectedItemIds])

  /** 保存当前勾选为 store 偏好 */
  const persist = useCallback(async (categories: MoleCleanCategory[]) => {
    const store = storeRef.current
    if (!store) return
    const selections: Record<string, boolean> = {}
    for (const cat of categories) {
      for (const item of cat.items) {
        const key = selKey(item.categoryId || cat.id, item.id)
        selections[key] = selectedItemIds.has(key)
      }
    }
    try {
      await store.set('clean_selections', selections)
      await store.save()
    } catch { /* ignore */ }
  }, [selectedItemIds])

  return {
    selectedItemIds,
    resetForNewScan,
    initializeFromScan,
    toggleItem,
    toggleGroup,
    resetToDefault,
    hasChangedFromDefault,
    persist,
  }
}
