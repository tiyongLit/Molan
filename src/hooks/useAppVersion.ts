import { useCallback, useEffect, useRef, useState } from 'react'
import useTauri, { EVT_APP_VERSION_PROGRESS } from '@/hooks/useTauri'
import type { AppVersionCheckResult } from '@/types/mole'

/** 检查结果缓存键（tauri-plugin-store） */
const STORE_KEY_LAST_CHECK = 'appVersion:lastCheckedAt'
const STORE_KEY_DISMISSED = 'appVersion:dismissedVersion'
const STORE_KEY_FAIL_COUNT = 'appVersion:consecutiveFailures'

/**
 * 启动后延迟静默检查（毫秒）。
 * 不立即检查：用户刚打开 app，不需要知道更新状态。
 * 5 分钟后才发起第一次静默检查。
 */
const SILENT_CHECK_DELAY_MS = 5 * 60_000

/**
 * 最小检查间隔（毫秒）：1 小时。
 * 仅用于失败退避上限的兜底基数；用户配置的节流间隔从 settings.json
 * 的 updateCheckInterval（1/7/30 天）读取。
 */
const MIN_CHECK_INTERVAL_MS = 60 * 60_000

/** 失败退避倍数：每次连续失败，下次间隔翻倍 */
const BACKOFF_MULTIPLIER = 2

/** 失败退避上限（毫秒）：8 小时 */
const MAX_BACKOFF_MS = 8 * 60 * 60_000

/** 周期性检查间隔（毫秒）：4 小时，仅作为"到点再查"的保底轮询 */
const PERIODIC_CHECK_INTERVAL_MS = 4 * 60 * 60_000

/** 从 settings.json 读取 autoCheckUpdate 开关 */
async function readAutoCheckUpdate(): Promise<boolean> {
  try {
    const { load } = await import('@tauri-apps/plugin-store')
    const s = await load('settings.json', { autoSave: true })
    const val = await s.get<boolean>('autoCheckUpdate')
    if (typeof val === 'boolean') return val
  } catch {
    // 降级
  }
  return true
}

/**
 * 从 settings.json 读取 updateCheckInterval（天），转为毫秒。
 * 合法值：1 / 7 / 30；不合法时降级为 1 天。
 */
async function readUpdateIntervalMs(): Promise<number> {
  try {
    const { load } = await import('@tauri-apps/plugin-store')
    const s = await load('settings.json', { autoSave: true })
    const val = await s.get<number>('updateCheckInterval')
    if (val === 1 || val === 7 || val === 30) {
      return val * 24 * 60 * 60_000
    }
  } catch {
    // 降级
  }
  return 1 * 24 * 60 * 60_000
}

export interface AppVersionState {
  /** 是否正在检查 */
  checking: boolean
  /** 检查结果 */
  result: AppVersionCheckResult | null
  /** 下载进度 0-100 */
  progress: number
  /** 是否正在安装 */
  installing: boolean
  /** 错误信息 */
  error: string | null
}

/**
 * MoleStudio 应用版本检查 hook。
 *
 * 封装 mole_app_version_check / mole_app_version_install 命令，
 * 提供检查、安装、进度追踪和结果缓存能力。
 *
 * 调度策略（VS Code 式后台静默检查）：
 * - 启动 5 分钟后首次静默检查（不阻塞用户操作）
 * - 持久化 lastCheckedAt 到 store，重启不重查
 * - 检查失败后间隔翻倍（1h → 2h → 4h → 8h 上限）
 * - 每 4 小时周期性检查（覆盖长会话场景）
 * - 用户手动点「检查更新」立即触发（绕过间隔）
 */
export function useAppVersion() {
  const tauri = useTauri()
  const [state, setState] = useState<AppVersionState>({
    checking: false,
    result: null,
    progress: 0,
    installing: false,
    error: null,
  })

  const lastCheckAtRef = useRef<number>(0)
  const consecutiveFailuresRef = useRef<number>(0)
  const dismissedVersionRef = useRef<string | null>(null)

  // 监听下载进度事件
  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<{ phase: string; progress: number }>(
      EVT_APP_VERSION_PROGRESS,
      (payload) => {
        setState((prev) => ({
          ...prev,
          progress: payload.progress,
          installing: payload.phase === 'installing',
        }))
      },
      ac.signal
    )
    return () => {
      ac.abort()
    }
  }, [tauri])

  // 从 store 恢复 dismissed version + lastCheckAt + failCount
  useEffect(() => {
    ;(async () => {
      try {
        const store = await import('@tauri-apps/plugin-store')
        const s = await store.load('settings.json', { autoSave: true })
        const dismissed = await s.get<string>(STORE_KEY_DISMISSED)
        if (dismissed) dismissedVersionRef.current = dismissed

        // 恢复上次检查时间（持久化，重启不重查）
        const lastCheckISO = await s.get<string>(STORE_KEY_LAST_CHECK)
        if (lastCheckISO) {
          const parsed = new Date(lastCheckISO).getTime()
          if (!isNaN(parsed)) lastCheckAtRef.current = parsed
        }

        // 恢复连续失败次数（用于退避计算）
        const failCount = await s.get<number>(STORE_KEY_FAIL_COUNT)
        if (typeof failCount === 'number') consecutiveFailuresRef.current = failCount
      } catch {
        // store 不可用时静默降级
      }
    })()
  }, [])

  /** 检查更新。`force=true` 绕过最小间隔限制（用户手动触发）。 */
  const checkForUpdate = useCallback(
    async (force = false) => {
      const now = Date.now()

      // 计算有效间隔（含失败退避）
      const backoffMultiplier = Math.pow(BACKOFF_MULTIPLIER, consecutiveFailuresRef.current)
      const effectiveInterval = Math.min(
        MIN_CHECK_INTERVAL_MS * backoffMultiplier,
        MAX_BACKOFF_MS
      )

      if (!force && now - lastCheckAtRef.current < effectiveInterval) {
        return
      }

      setState((prev) => ({ ...prev, checking: true, error: null }))
      try {
        const result = await tauri.mole_app_version_check() as AppVersionCheckResult
        lastCheckAtRef.current = Date.now()
        consecutiveFailuresRef.current = 0

        // 持久化检查时间 + 重置失败计数
        try {
          const store = await import('@tauri-apps/plugin-store')
          const s = await store.load('settings.json', { autoSave: true })
          await s.set(STORE_KEY_LAST_CHECK, new Date().toISOString())
          await s.set(STORE_KEY_FAIL_COUNT, 0)
        } catch {
          // 降级
        }

        setState((prev) => ({
          ...prev,
          checking: false,
          result,
          progress: 0,
          installing: false,
        }))
      } catch (e: unknown) {
        consecutiveFailuresRef.current += 1

        // 持久化失败计数（下次启动时继续退避）
        try {
          const store = await import('@tauri-apps/plugin-store')
          const s = await store.load('settings.json', { autoSave: true })
          await s.set(STORE_KEY_FAIL_COUNT, consecutiveFailuresRef.current)
        } catch {
          // 降级
        }

        setState((prev) => ({
          ...prev,
          checking: false,
          error: e instanceof Error ? e.message : String(e),
        }))
      }
    },
    [tauri]
  )

  /** 安装更新（官网版：下载安装；MAS 版：打开 App Store） */
  const installUpdate = useCallback(async () => {
    const result = state.result
    if (!result) return

    if (result.source === 'app_store') {
      try {
        await tauri.mole_app_version_open_appstore()
      } catch (e: unknown) {
        setState((prev) => ({
          ...prev,
          error: e instanceof Error ? e.message : String(e),
        }))
      }
      return
    }

    // 官网版：下载安装
    setState((prev) => ({ ...prev, installing: true, progress: 0 }))
    try {
      await tauri.mole_app_version_install()
      // 如果到这里说明安装成功即将重启，无需更新 UI
    } catch (e: unknown) {
      setState((prev) => ({
        ...prev,
        installing: false,
        error: e instanceof Error ? e.message : String(e),
      }))
    }
  }, [state.result, tauri])

  /** 忽略当前版本（不再提示红点） */
  const dismissVersion = useCallback(async () => {
    const version = state.result?.latest_version
    if (!version) return
    dismissedVersionRef.current = version
    try {
      const store = await import('@tauri-apps/plugin-store')
      const s = await store.load('settings.json', { autoSave: true })
      await s.set(STORE_KEY_DISMISSED, version)
    } catch {
      // 降级
    }
  }, [state.result?.latest_version])

  /** 是否有新版本（排除已忽略的版本） */
  const hasUpdate =
    state.result?.available === true &&
    state.result?.latest_version !== dismissedVersionRef.current

  // ── 静默检查调度：启动 5 分钟后首次检查 + 每 4 小时周期检查 ──
  // 读取 settings.json 中的 autoCheckUpdate 与 updateCheckInterval：
  //   - autoCheckUpdate=false → 不启动任何静默定时器（仅保留手动检查入口）
  //   - updateCheckInterval（1/7/30 天）作为节流间隔
  //   - 首次检查不早于启动后 5 分钟
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let periodic: ReturnType<typeof setInterval> | undefined
    let cancelled = false

    const schedule = async () => {
      // 先等 store 恢复完成（给上面的恢复 effect 一帧时间）
      await new Promise((r) => setTimeout(r, 100))

      // 读用户偏好：开关 + 节流间隔（天）
      const autoCheck = await readAutoCheckUpdate()
      if (cancelled || !autoCheck) return

      const intervalMs = await readUpdateIntervalMs()
      if (cancelled) return

      const now = Date.now()
      const elapsed = now - lastCheckAtRef.current
      const remaining = lastCheckAtRef.current > 0
        ? Math.max(0, intervalMs - elapsed)
        : 0

      // 首次静默检查：取 max(5分钟启动延迟, 间隔剩余时间)
      const initialDelay = remaining > 0
        ? Math.max(remaining, SILENT_CHECK_DELAY_MS)
        : SILENT_CHECK_DELAY_MS

      timer = setTimeout(() => {
        checkForUpdate(false)
        // 首次检查完成后，启动 4 小时周期检查
        periodic = setInterval(() => {
          checkForUpdate(false)
        }, PERIODIC_CHECK_INTERVAL_MS)
      }, initialDelay)
    }

    schedule()

    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
      if (periodic) clearInterval(periodic)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return {
    state,
    hasUpdate,
    checkForUpdate,
    installUpdate,
    dismissVersion,
  }
}
