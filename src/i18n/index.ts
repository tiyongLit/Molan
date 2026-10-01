import { useSyncExternalStore } from 'react'
import zhCN from './locales/zh-CN/translation.json'
import enUS from './locales/en-US/translation.json'
import zhTW from './locales/zh-TW/translation.json'

/** 支持的语言：英文 / 简体中文 / 繁体中文；zh-CN 为 key 口径基准（缺 key 时回退） */
export type AppLocale = 'zh-CN' | 'en-US' | 'zh-TW'

export const SUPPORTED_LOCALES: AppLocale[] = ['en-US', 'zh-CN', 'zh-TW']

/** 语言选项固定用母语名（endonym），不随界面语言翻译 */
export const LOCALE_LABELS: Record<AppLocale, string> = {
  'zh-CN': '简体中文',
  'en-US': 'English',
  'zh-TW': '繁體中文',
}

type TranslationDict = Record<string, string>

/** 取词 key 联合类型：以 zh-CN 字典为准（目标语言缺 key 时回退 zh-CN） */
export type TranslationKey = keyof typeof zhCN

const DICTS: Record<AppLocale, TranslationDict> = {
  'zh-CN': zhCN,
  'en-US': enUS,
  'zh-TW': zhTW,
}

// localStorage 仅作跨窗口同步水合缓存（各窗口模块加载时同步读取，避免语言闪烁）；
// 事实源是 settings.json store 的 language 字段，由 useSettings 读写并回填这里。
const STORAGE_KEY = 'mole.locale'

function isSupportedLocale(v: unknown): v is AppLocale {
  return typeof v === 'string' && (SUPPORTED_LOCALES as string[]).includes(v)
}

/** 首次运行按系统语言探测；无法识别时回退简体中文（保持旧行为） */
function detectSystemLocale(): AppLocale {
  const lang = navigator.language || ''
  if (lang.startsWith('zh')) {
    // zh-TW / zh-HK / zh-Hant* → 繁体，其余中文 → 简体
    if (/^(zh-TW|zh-HK|zh-MO|zh-Hant)/i.test(lang)) return 'zh-TW'
    return 'zh-CN'
  }
  if (lang.startsWith('en')) return 'en-US'
  return 'zh-CN'
}

let currentLocale: AppLocale = detectSystemLocale()
try {
  const stored = localStorage.getItem(STORAGE_KEY)
  if (isSupportedLocale(stored)) currentLocale = stored
} catch {
  // localStorage 不可用时用系统语言
}

const listeners = new Set<() => void>()

export function getLocale(): AppLocale {
  return currentLocale
}

/** 应用语言并通知订阅者（不写缓存）；同值早退，保证幂等、避免跨窗口回环 */
function applyLocale(next: AppLocale) {
  if (!isSupportedLocale(next) || next === currentLocale) return
  currentLocale = next
  listeners.forEach((fn) => fn())
}

/** 切换语言：更新运行时 + 写同步缓存 + 通知订阅组件重渲染 */
export function setLocale(next: AppLocale) {
  if (!isSupportedLocale(next) || next === currentLocale) return
  applyLocale(next)
  try {
    localStorage.setItem(STORAGE_KEY, next)
  } catch {
    // 缓存写入失败不影响本次会话的语言切换
  }
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

// 跨窗口同步：其他窗口改语言会触发本窗口的 storage 事件（同源多 webview 共享 localStorage）。
// 只应用不回写（applyLocale），避免窗口间来回写导致的回环。
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === STORAGE_KEY && isSupportedLocale(e.newValue)) applyLocale(e.newValue)
  })
}

/** 从 settings store 水合语言（其他窗口/启动时调用）；无值时把探测结果写回 store */
export async function syncLocaleFromStore(): Promise<void> {
  try {
    const { load } = await import('@tauri-apps/plugin-store')
    const store = await load('settings.json', { autoSave: false })
    const val = await store.get('language')
    if (isSupportedLocale(val)) {
      setLocale(val)
    } else if (val === undefined || val === null) {
      await store.set('language', currentLocale)
      await store.save()
    }
  } catch {
    // store 不可用（如权限缺失）时静默跳过，保留 localStorage / 系统语言
  }
}

/** 参数化插值：文案中用 `{{name}}` 占位，调用时传同名参数即可替换 */
export type TranslationParams = Record<string, string | number>

/** 取词函数签名：useI18n 返回值与传入纯函数（如 statusChip）的 t 均为该类型 */
export type TFunction = (key: TranslationKey, params?: TranslationParams) => string

function interpolate(template: string, params?: TranslationParams): string {
  if (!params) return template
  return template.replace(/\{\{(\w+)\}\}/g, (_, key) => {
    const v = params[key]
    return v === undefined ? `{{${key}}}` : String(v)
  })
}

/** 类型化取词：key 以 zh-CN 字典为准，目标语言缺 key 时回退 zh-CN；支持 `{{name}}` 插值 */
export function t(key: TranslationKey, params?: TranslationParams): string {
  const template = DICTS[currentLocale][key] ?? zhCN[key]
  return interpolate(template, params)
}

/** 组件内订阅语言变化：语言切换后返回的 t 会取到新语言文案并触发重渲染 */
export function useI18n(): { locale: AppLocale; t: TFunction } {
  const locale = useSyncExternalStore(subscribe, getLocale, getLocale)
  return { locale, t }
}

export function trashReminderError(code: string): string {
  if (code.includes('SAVE_FAILED')) return t('trashReminder.saveFailed')
  if (code.includes('BUSY')) return t('trashReminder.busy')
  if (code.includes('UNREADABLE')) return t('trashReminder.readFailed')
  if (code.includes('CONFIG')) return t('trashReminder.configFailed')
  return t('trashReminder.emptyFailed')
}
