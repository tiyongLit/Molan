import { useEffect, useRef } from 'react'
import { useMemoizedFn } from 'ahooks'
import { EVT_TABLE_REFRESH } from '@/constants/tauri-events'
import useTauri from '@/hooks/useTauri'
import type { TableRefreshPayload } from '@/types'

/** 广播 `keys` 与 `tableKeys` 有交集时触发 `onRefresh`（多窗口各自订阅） */
export function useTableRefreshListener(tableKeys: string[], onRefresh: () => void) {
  const { onIpcEvent } = useTauri()
  const keysRef = useRef(tableKeys)
  keysRef.current = tableKeys
  const run = useMemoizedFn(onRefresh)

  useEffect(() => {
    const ac = new AbortController()
    onIpcEvent<TableRefreshPayload>(
      EVT_TABLE_REFRESH,
      (payload) => {
        if (!payload?.keys?.length) return
        const want = keysRef.current
        if (!want.some((k) => payload.keys.includes(k))) return
        run()
      },
      ac.signal
    )
    return () => ac.abort()
  }, [onIpcEvent, run])
}
