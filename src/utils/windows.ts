import { invoke } from '@tauri-apps/api/core'

export const DEFAULT_WINDOW_WIDTH = 383
export const DEFAULT_WINDOW_HEIGHT = 618

export async function smoothResizeWindow(
  targetWidth: number,
  targetHeight: number,
  duration: number = 300,
  steps: number = 30,
  signal?: AbortSignal
): Promise<void> {
  if (signal?.aborted) return

  const { width: currentWidth, height: currentHeight } = await invoke<{
    width: number
    height: number
  }>('get_window_size')

  if (signal?.aborted) return

  const stepDuration = duration / steps

  for (let i = 0; i <= steps; i++) {
    if (signal?.aborted) return
    const t = i / steps
    const w = Math.round(currentWidth + (targetWidth - currentWidth) * t)
    const h = Math.round(currentHeight + (targetHeight - currentHeight) * t)
    try {
      await invoke('set_window_size', { width: w, height: h })
    } catch {
      return
    }
    try {
      await new Promise<void>((resolve) => {
        const tid = setTimeout(resolve, stepDuration)
        const onAbort = () => {
          clearTimeout(tid)
          resolve()
        }
        signal?.addEventListener('abort', onAbort, { once: true })
      })
    } catch {
      return
    }
  }
}
