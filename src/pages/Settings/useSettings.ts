import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  CMD_MOLE_AUTO_LAUNCH_STATUS,
  CMD_MOLE_AUTO_LAUNCH_TOGGLE,
  CMD_MOLE_TRASH_REMINDER_GET_STATE,
  CMD_MOLE_TRASH_REMINDER_UPDATE_SETTINGS,
} from '@/constants/tauri-commands'

import type { TrashReminderSnapshot } from '@/types/mole'
import { trashReminderError, getLocale, setLocale, SUPPORTED_LOCALES } from '@/i18n'
import type { AppLocale } from '@/i18n'

/** 设置页初始值；废纸篓提醒的生效配置由 Rust 返回。 */
export const SETTINGS_DEFAULTS = {
  /** 界面语言；默认取 i18n 运行时的系统探测 / localStorage 水合值 */
  language: getLocale(),
  autoLaunch: true,
  autoCheckUpdate: true,
  updateCheckInterval: 1,
  clean: {
    deleteMode: 'trash' as DeleteMode,
    sizeMetric: 'logical' as SizeMetric,
  },
  uninstall: {
    autoDetectResidual: true,
    historyRetention: 30,
    showSystemApps: false,
  },
  trashReminder: {
    enabled: true,
    /** 废纸篓达到此大小（MB）时在右上角提醒；默认 4 GB */
    threshold: 4096,
  },
  dashboard: {
    /** Dashboard 内存列表 hover 时是否显示关闭进程按钮 */
    enableKillProcess: true,
  },
}

export type DeleteMode = 'trash' | 'direct'
export type SizeMetric = 'logical' | 'physical'

export interface AppSettings {
  language: AppLocale
  autoLaunch: boolean
  autoCheckUpdate: boolean
  updateCheckInterval: number
  clean: {
    deleteMode: DeleteMode
    sizeMetric: SizeMetric
  }
  uninstall: {
    autoDetectResidual: boolean
    historyRetention: number
    showSystemApps: boolean
  }
  trashReminder: {
    enabled: boolean
    threshold: number
  }
  dashboard: {
    enableKillProcess: boolean
  }
}

/** 类型守卫：判断是否为普通对象（store 读回的嵌套设置） */
function isRecord(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object'
}

/** 类型守卫：判断 store 读回的值是否为受支持的语言 */
function isAppLocale(v: unknown): v is AppLocale {
  return typeof v === 'string' && (SUPPORTED_LOCALES as string[]).includes(v)
}

/** 把 store 读回的通用设置值（逐个类型校验）合并进 base，返回新对象。
 * 与 init 的合并口径一致，供 visibilitychange 重读复用。 */
function mergeGeneral(base: AppSettings, raw: Record<string, unknown>): AppSettings {
  const next: AppSettings = { ...base }
  if (isAppLocale(raw.language)) {
    next.language = raw.language
    // 跨窗口/其他地方改过语言时，让 i18n 运行时一并跟进（setLocale 同值早退，幂等）
    setLocale(raw.language)
  }
  if (typeof raw.autoCheckUpdate === 'boolean') next.autoCheckUpdate = raw.autoCheckUpdate
  if (typeof raw.updateCheckInterval === 'number') next.updateCheckInterval = raw.updateCheckInterval
  if (isRecord(raw.clean)) {
    const dm = raw.clean.deleteMode
    const sm = raw.clean.sizeMetric
    if (dm === 'trash' || dm === 'direct') next.clean = { ...next.clean, deleteMode: dm }
    if (sm === 'logical' || sm === 'physical') next.clean = { ...next.clean, sizeMetric: sm }
  }
  if (isRecord(raw.uninstall)) {
    const adr = raw.uninstall.autoDetectResidual
    const hr = raw.uninstall.historyRetention
    const ssa = raw.uninstall.showSystemApps
    if (typeof adr === 'boolean') next.uninstall = { ...next.uninstall, autoDetectResidual: adr }
    if (typeof hr === 'number') next.uninstall = { ...next.uninstall, historyRetention: hr }
    if (typeof ssa === 'boolean') next.uninstall = { ...next.uninstall, showSystemApps: ssa }
  }
  if (isRecord(raw.dashboard)) {
    const ekp = raw.dashboard.enableKillProcess
    if (typeof ekp === 'boolean') next.dashboard = { ...next.dashboard, enableKillProcess: ekp }
  }
  return next
}

/**
 * 设置读写 hook：封装 tauri-plugin-store 的 settings.json。
 *
 * - 挂载时从 store 读取全部设置，与默认值合并
 * - setSetting 立即写入 store 并更新本地状态
 * - autoLaunch 走 Rust 命令（需要操作系统级 LoginItems 交互）
 */
export function useSettings() {
  const [settings, setSettings] = useState<AppSettings>({ ...SETTINGS_DEFAULTS })
  const [loading, setLoading] = useState(true)
  const [trashSaving, setTrashSaving] = useState(false)
  const [trashError, setTrashError] = useState('')
  const trashWriting = useRef(false)
  const trashRevision = useRef(-1)
  const settingsRef = useRef(settings)
  settingsRef.current = settings
  const storeRef = useRef<import('@tauri-apps/plugin-store').Store | null>(null)

  // 初始化：加载 store + 查询 auto_launch 状态
  useEffect(() => {
    let cancelled = false

    ;(async () => {
      try {
        const snap = await invoke<TrashReminderSnapshot>(CMD_MOLE_TRASH_REMINDER_GET_STATE)
        if (cancelled) return
        trashRevision.current = snap.revision
        setSettings(prev => ({ ...prev, trashReminder: { enabled: snap.enabled, threshold: snap.thresholdMB } }))
        setTrashError(snap.errorCode ? trashReminderError(snap.errorCode) : '')
        if (snap.errorCode?.includes('CONFIG')) {
          // 不触发 Store 默认值及 autoLaunch 的初始化写回，保留损坏文件供恢复。
          setLoading(false)
          return
        }
        const { load } = await import('@tauri-apps/plugin-store')
        const store = await load('settings.json', { autoSave: false })
        if (cancelled) return
        storeRef.current = store

        // 读取所有设置（与默认值合并，缺失字段用默认值填充）
        const stored: Record<string, unknown> = {}
        for (const key of Object.keys(SETTINGS_DEFAULTS)) {
          if (key === 'trashReminder') continue
          const val = await store.get(key)
          if (val !== undefined && val !== null) {
            stored[key] = val
          }
        }

        const merged = { ...SETTINGS_DEFAULTS }
        if (isAppLocale(stored.language)) {
          merged.language = stored.language
          setLocale(stored.language)
        }
        if (typeof stored.autoCheckUpdate === 'boolean') {
          merged.autoCheckUpdate = stored.autoCheckUpdate
        }
        if (typeof stored.updateCheckInterval === 'number') {
          merged.updateCheckInterval = stored.updateCheckInterval
        }
        if (stored.clean && typeof stored.clean === 'object') {
          const c = stored.clean as Record<string, unknown>
          if (c.deleteMode === 'trash' || c.deleteMode === 'direct') {
            merged.clean = { ...merged.clean, deleteMode: c.deleteMode as DeleteMode }
          }
          if (c.sizeMetric === 'logical' || c.sizeMetric === 'physical') {
            merged.clean = { ...merged.clean, sizeMetric: c.sizeMetric as SizeMetric }
          }
        }
        if (stored.uninstall && typeof stored.uninstall === 'object') {
          const u = stored.uninstall as Record<string, unknown>
          if (typeof u.autoDetectResidual === 'boolean') {
            merged.uninstall = { ...merged.uninstall, autoDetectResidual: u.autoDetectResidual }
          }
          if (typeof u.historyRetention === 'number') {
            merged.uninstall = { ...merged.uninstall, historyRetention: u.historyRetention }
          }
          if (typeof u.showSystemApps === 'boolean') {
            merged.uninstall = { ...merged.uninstall, showSystemApps: u.showSystemApps }
          }
        }
        if (stored.dashboard && typeof stored.dashboard === 'object') {
          const d = stored.dashboard as Record<string, unknown>
          if (typeof d.enableKillProcess === 'boolean') {
            merged.dashboard = { ...merged.dashboard, enableKillProcess: d.enableKillProcess }
          }
        }
        merged.trashReminder = { enabled: snap.enabled, threshold: snap.thresholdMB }
        if (cancelled) return

        // autoLaunch：以 store 为准；首次运行（store 无值）从 OS LoginItems 读一次作为初始值。
        // 无论本次是否命中，都按 store 最终值反向同步 OS（用户可能在系统设置里动过登录项）。
        let autoLaunchVal = merged.autoLaunch
        const storedAutoLaunch = stored.autoLaunch
        if (typeof storedAutoLaunch === 'boolean') {
          autoLaunchVal = storedAutoLaunch
        } else {
          // store 未设置过：从 OS 读一次写入 store，作为默认
          try {
            autoLaunchVal = await invoke<boolean>(CMD_MOLE_AUTO_LAUNCH_STATUS)
          } catch {
            autoLaunchVal = merged.autoLaunch
          }
        }
        merged.autoLaunch = autoLaunchVal
        await persistToStore('autoLaunch', autoLaunchVal)
        // 语言：store 无值时把当前（系统探测 / localStorage）值落盘，作为默认
        if (!isAppLocale(stored.language)) {
          await persistToStore('language', merged.language)
        }
        // 按偏好同步 OS 状态（store 是事实源）
        try {
          await invoke(CMD_MOLE_AUTO_LAUNCH_TOGGLE, { enable: autoLaunchVal })
        } catch {
          // OS 同步失败不阻塞 UI
        }

        if (!cancelled) {
          setSettings(prev => ({ ...merged, trashReminder: prev.trashReminder }))
          setLoading(false)
        }
      } catch {
        if (!cancelled) {
          setSettings(prev => ({ ...prev, trashReminder: { ...prev.trashReminder, enabled: false } }))
          setTrashError(trashReminderError('TRASH_CONFIG_UNAVAILABLE'))
          setLoading(false)
        }
      }
    })()

    return () => { cancelled = true }
  }, [])

  // 废纸篓提醒：窗口获焦时从 Rust 同步最新状态
  useEffect(() => {
    let disposed = false
    const reconcile = () => {
      if (trashWriting.current) return
      void invoke<TrashReminderSnapshot>(CMD_MOLE_TRASH_REMINDER_GET_STATE).then(snap => {
        if (disposed || trashWriting.current || snap.revision < trashRevision.current) return
        trashRevision.current = snap.revision
        setSettings(prev => ({ ...prev, trashReminder: { enabled: snap.enabled, threshold: snap.thresholdMB } }))
        setTrashError(snap.errorCode ? trashReminderError(snap.errorCode) : '')
      }).catch(() => { if (!disposed) setTrashError(trashReminderError('TRASH_CONFIG_UNAVAILABLE')) })
    }
    window.addEventListener('focus', reconcile)
    return () => { disposed = true; window.removeEventListener('focus', reconcile) }
  }, [])

  // 确保 store 已加载（懒加载兜底）：即使 init 因 trashReminder 命令失败/早退
  // 而未赋值 storeRef，这里也能独立加载，保证通用设置始终可读。
  const ensureStore = useCallback(async () => {
    if (storeRef.current) return storeRef.current
    const { load } = await import('@tauri-apps/plugin-store')
    const store = await load('settings.json', { autoSave: false })
    storeRef.current = store
    return store
  }, [])

  // 从 store 重新读取通用设置（不含 trashReminder/autoLaunch，它们走 Rust 命令）。
  const reloadGeneralFromStore = useCallback(async () => {
    try {
      const store = await ensureStore()
      const [language, autoCheckUpdate, updateCheckInterval, clean, uninstall, dashboard] = await Promise.all([
        store.get('language'),
        store.get('autoCheckUpdate'),
        store.get('updateCheckInterval'),
        store.get('clean'),
        store.get('uninstall'),
        store.get('dashboard'),
      ])
      setSettings(prev => mergeGeneral(prev, { language, autoCheckUpdate, updateCheckInterval, clean, uninstall, dashboard }))
    } catch (e) {
      console.error('[Settings] reloadGeneralFromStore failed:', e)
    }
  }, [ensureStore])

  // 跨窗口设置同步：Dashboard 是隐藏托盘气泡窗，Settings 改值写入 store 后它无法实时感知。
  // 对齐本仓库既有可靠模式（AuthBanner / useStatusSnapshot）：监听 visibilitychange，
  // 托盘重新打开（文档变可见）时从 store 重读，保证最终一致。
  // 注：不用前端 emit/listen 跨窗广播——隐藏 webview 期间事件可能丢失，visibilitychange 才是本窗口验证过的可靠信号。
  useEffect(() => {
    const onVis = () => { if (document.visibilityState === 'visible') void reloadGeneralFromStore() }
    document.addEventListener('visibilitychange', onVis)
    return () => document.removeEventListener('visibilitychange', onVis)
  }, [reloadGeneralFromStore])

  /** 写入单个设置项到 store */
  const persistToStore = useCallback(async (key: string, value: unknown) => {
    const store = storeRef.current
    if (!store || key === 'trashReminder') return
    try {
      await store.set(key, value)
      await store.save()
    } catch (e) {
      console.error('[Settings] persistToStore failed:', { key, value, error: e })
    }
  }, [])

  /** 通用设置更新（自动 merge 嵌套对象 + 持久化） */
  const updateSetting = useCallback(<K extends keyof AppSettings>(
    key: K,
    value: AppSettings[K] | ((prev: AppSettings[K]) => AppSettings[K])
  ) => {
    if (key === 'trashReminder') {
      if (trashWriting.current) return
      const next = (typeof value === 'function' ? value(settingsRef.current[key]) : value) as AppSettings['trashReminder']
      trashWriting.current = true
      setTrashSaving(true)
      setTrashError('')
      void invoke<TrashReminderSnapshot>(CMD_MOLE_TRASH_REMINDER_UPDATE_SETTINGS, { args: next }).then(snap => {
        if (snap.revision < trashRevision.current) return
        trashRevision.current = snap.revision
        setSettings(prev => ({ ...prev, trashReminder: { enabled: snap.enabled, threshold: snap.thresholdMB } }))
      }).catch(error => setTrashError(trashReminderError(String(error))))
        .finally(() => { trashWriting.current = false; setTrashSaving(false) })
      return
    }
    setSettings(prev => {
      const newValue = typeof value === 'function'
        ? (value as (prev: AppSettings[K]) => AppSettings[K])(prev[key])
        : value
      const next = { ...prev, [key]: newValue }
      // 异步持久化（不阻塞状态更新）
      persistToStore(key, newValue)
      return next
    })
  }, [persistToStore])

  /** 切换开机自启动（同步 OS LoginItems + store + 本地状态） */
  const toggleAutoLaunch = useCallback(async (enable: boolean) => {
    try {
      await invoke(CMD_MOLE_AUTO_LAUNCH_TOGGLE, { enable })
      setSettings(prev => ({ ...prev, autoLaunch: enable }))
      await persistToStore('autoLaunch', enable)
    } catch (e) {
      console.error('[Settings] auto launch toggle failed:', e)
    }
  }, [persistToStore])

  /** 切换界面语言：同步 i18n 运行时（立即重渲染）+ store（持久化）+ 本地状态 */
  const changeLanguage = useCallback((locale: AppLocale) => {
    setLocale(locale)
    setSettings(prev => ({ ...prev, language: locale }))
    void persistToStore('language', locale)
  }, [persistToStore])

  return {
    settings,
    loading,
    trashSaving,
    trashError,
    updateSetting,
    toggleAutoLaunch,
    changeLanguage,
  }
}
