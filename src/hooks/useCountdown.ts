import { useEffect, useState } from 'react'

function formatHms(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  return [h, m, sec].map((n) => String(n).padStart(2, '0')).join(':')
}

function remainingSeconds(endMs: number): number {
  return Math.floor((endMs - Date.now()) / 1000)
}

/**
 * 每秒刷新一次到 `endTimestampMs` 的剩余时间（HH:mm:ss），不请求后端。
 * `endTimestampMs === null` 时固定返回 `placeholder`。
 */
export function useCountdown(endTimestampMs: number | null, placeholder: string): string {
  const [text, setText] = useState(() =>
    endTimestampMs == null ? placeholder : formatHms(remainingSeconds(endTimestampMs))
  )

  useEffect(() => {
    if (endTimestampMs == null) {
      setText(placeholder)
      return
    }
    const tick = () => setText(formatHms(remainingSeconds(endTimestampMs)))
    tick()
    const id = window.setInterval(tick, 1000)
    return () => window.clearInterval(id)
  }, [endTimestampMs, placeholder])

  return text
}
