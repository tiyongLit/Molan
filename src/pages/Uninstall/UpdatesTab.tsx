import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { AppWindow, Package, CheckCircle2 } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import useTauri, { EVT_UPDATES_BREW_PROGRESS, EVT_UPDATES_INSTALL_PROGRESS } from '@/hooks/useTauri'
import { moleMessage } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { formatSize } from '@/utils/format'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { SectionHeader } from '@/components/business/Apps/SectionHeader'
import { Badge } from '@/components/business/Apps/Badge'
import { useI18n } from '@/i18n'
import type {
  AppCheckResult,
  BrewOutdatedItem,
  BrewProgressEvent,
  InstallProgressEvent,
  InstallPrepareResult,
  MoleListAppsEntry,
} from '@/types/mole'

// ── 来源 badge ──
const SOURCE_BADGE: Record<string, string> = {
  sparkle: 'Sparkle',
  app_store: 'App Store',
  electron: 'Electron',
  homebrew: 'Homebrew',
}

const SOURCE_STYLE: Record<string, { color: string; bg: string }> = {
  sparkle: { color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  app_store: { color: '#a3a3a3', bg: 'rgba(255,255,255,0.08)' },
  electron: { color: '#8b5cf6', bg: 'rgba(139,92,246,0.12)' },
  homebrew: { color: '#fbbf24', bg: 'rgba(251,191,36,0.12)' },
}

// ── 原地安装（更新执行引擎）UI 状态 ──

/** 与后端 `engine::session::InstallStage` 对应的前端阶段 */
type InstallUiStage = 'downloading' | 'verifying' | 'ready' | 'installing' | 'completed'

interface InstallUi {
  stage: InstallUiStage
  /** 下载阶段的已下载字节数 */
  bytes: number
  /** ready / completed 时的新版本号 */
  newVersion?: string
}

/** 会话级 guard：自动 surface 每会话一次 */
let brewSurfaced = false

/**
 * 版本比较：对齐 lib/updates/version.rs::is_version_newer：
 * trim → 剥一个前导 v/V → 点分 → 每段整数（失败归 0）→ 缺段补 0 逐段比 → 全等 false。
 */
function isVersionNewer(remote: string, local: string): boolean {
  const parts = (v: string) =>
    v
      .trim()
      .replace(/^v/i, '')
      .split('.')
      .map((s) => {
        const n = parseInt(s, 10)
        return Number.isNaN(n) ? 0 : n
      })
  const a = parts(remote)
  const b = parts(local)
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0
    const y = b[i] ?? 0
    if (x !== y) return x > y
  }
  return false
}

/**
 * 系统兼容门 —— minimum 空 → 可安装；
 * 否则 running >= minimum（每段取前导数字，缺段补 0，全等满足）。
 */
function osIsInstallable(minimumOS: string | null | undefined, runningOS: string): boolean {
  const m = (minimumOS ?? '').trim()
  if (!m) return true
  const parse = (v: string) =>
    v.split('.').map((s) => {
      const head = s.match(/^\d+/)
      return head ? parseInt(head[0], 10) : 0
    })
  const a = parse(runningOS)
  const b = parse(m)
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0
    const y = b[i] ?? 0
    if (x !== y) return x > y
  }
  return true
}

/** 陈旧判定：从未打开或 >30 天未用 → amber 高亮 */
function isStale(lastUsedEpoch: number): boolean {
  if (!lastUsedEpoch) return true
  return Date.now() / 1000 - lastUsedEpoch > 30 * 86_400
}

function sourceChip(source: string) {
  const label = SOURCE_BADGE[source] ?? source
  const s = SOURCE_STYLE[source] ?? SOURCE_STYLE.sparkle
  return <Badge label={label} color={s.color} bg={s.bg} />
}

/**
 * app 行（available / up to date / mechanism / not checkable 四区共用）：
 * 图标 + 名称 + 来源 badge + 版本/大小/最近使用 meta + 可选右侧操作列。
 */
function AppUpdateRow({
  app,
  latestVersion,
  action,
  compact = false,
}: {
  app: MoleListAppsEntry
  /** 传入时 meta 显示 "v旧 → v新" 箭头（available 区） */
  latestVersion?: string
  /** 右侧操作列（不传则不渲染该列） */
  action?: ReactNode
  /** 紧凑变体：无来源 badge、单行 meta、不显示最近使用（not checkable 区） */
  compact?: boolean
}) {
  return (
    <div className="flex items-center gap-3 px-[24px] py-2">
      <AppIcon name={app.display_name || app.name} path={app.path} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
            {app.display_name || app.name}
          </span>
          {!compact && sourceChip(app.update_source ?? '')}
        </div>
        {compact ? (
          <div className="text-[10px] font-mono text-white/60">
            v{app.version} · {app.size_human}
          </div>
        ) : (
          <div className="flex items-center gap-1 text-[10px] font-mono text-white/60">
            <span>{latestVersion ? `v${app.version} → v${latestVersion}` : `v${app.version}`}</span>
            <span>·</span>
            <span>{app.size_human}</span>
            <span>·</span>
            <span style={{ color: isStale(app.last_used_epoch) ? '#fbbf24' : undefined }}>
              {app.last_used_relative}
            </span>
          </div>
        )}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  )
}

/**
 * 更新 tab。
 *
 * 数据流：
 *   挂载 → autoSurface（brew outdated，每会话一次，仅空时写入）
 *   Check → mole_updates_check（并发 6：appcast/iTunes + brew 刷新，直接覆盖）
 *   行内 Update → mole_updates_apply 深链（sparkle/electron 开应用、App Store 开页面）
 *   brew Upgrade → mole_updates_brew_upgrade 流式（进度走 updates::brew-progress 事件）
 *
 * 分区语义：
 *   未 checked → 显示 "Apps with an update mechanism"（有更新机制的 app 全部）
 *   checked 后 → available / up to date；latest 为空的行消失（Electron/网络失败静默）
 *   "Not checkable"（update_source = null）永远显示在最后
 */
export function UpdatesTab({ apps }: { apps: MoleListAppsEntry[] }) {
  const tauri = useTauri()
  const { t } = useI18n()
  const [checked, setChecked] = useState(false)
  const [checking, setChecking] = useState(false)
  const [brewSurfacing, setBrewSurfacing] = useState(false)
  const [checkResults, setCheckResults] = useState<Map<string, AppCheckResult>>(new Map())
  const [runningOs, setRunningOs] = useState('')
  const [brewItems, setBrewItems] = useState<BrewOutdatedItem[]>([])
  const [upgrading, setUpgrading] = useState<Set<string>>(new Set())
  /** 全局升级进度短语（并发 guard 保证同时只有一个升级任务） */
  const [brewPhrase, setBrewPhrase] = useState('')
  /** 深链交接状态（对齐 Burrow `.handedOff`）：已成功把更新交接给目标 App / App Store 的行 */
  const [handedOff, setHandedOff] = useState<Map<string, 'app' | 'appstore'>>(new Map())
  /** 原地安装状态（更新执行引擎）：按 app.path 记录阶段/进度 */
  const [installs, setInstalls] = useState<Map<string, InstallUi>>(new Map())

  // ── 挂载：autoSurface（brew outdated，每会话一次）──
  useEffect(() => {
    if (brewSurfaced) return
    brewSurfaced = true
    setBrewSurfacing(true)
    tauri
      .mole_updates_brew_outdated()
      .then((items: BrewOutdatedItem[]) => {
        // 仅当当前为空时写入
        setBrewItems((prev) => (prev.length === 0 ? (items ?? []) : prev))
      })
      .catch(() => {})
      .finally(() => setBrewSurfacing(false))
  }, [tauri])

  // ── brew 升级流式进度事件 ──
  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<BrewProgressEvent>(
      EVT_UPDATES_BREW_PROGRESS,
      (payload) => setBrewPhrase(payload?.phrase ?? ''),
      ac.signal
    )
    return () => ac.abort()
  }, [tauri])

  // ── 原地安装流式进度事件（prepare/commit 阶段推进）──
  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<InstallProgressEvent>(
      EVT_UPDATES_INSTALL_PROGRESS,
      (payload) => {
        if (!payload?.app_path) return
        setInstalls((prev) => {
          const cur = prev.get(payload.app_path)
          const bytes = cur?.bytes ?? 0
          const newVersion = cur?.newVersion
          const next = new Map(prev)
          switch (payload.stage) {
            case 'downloading':
              next.set(payload.app_path, {
                stage: 'downloading',
                bytes: payload.bytes ?? 0,
                newVersion,
              })
              break
            case 'verifying':
              next.set(payload.app_path, { stage: 'verifying', bytes, newVersion })
              break
            case 'ready_to_install':
              next.set(payload.app_path, { stage: 'ready', bytes, newVersion })
              break
            case 'installing':
              next.set(payload.app_path, { stage: 'installing', bytes, newVersion })
              break
            case 'completed':
              next.set(payload.app_path, { stage: 'completed', bytes, newVersion })
              break
          }
          return next
        })
      },
      ac.signal
    )
    return () => ac.abort()
  }, [tauri])

  // ── 本地分区：有更新机制的 app / 不可检测的 app（对齐 prepare 的 detected/unknown）──
  const mechanismApps = useMemo(
    () => apps.filter((a) => a.update_source !== null),
    [apps]
  )
  const uncheckableApps = useMemo(
    () => apps.filter((a) => a.update_source === null),
    [apps]
  )

  // ── checked 后的 available / up to date（对齐 availableItems / upToDateItems）──
  const { available, upToDate } = useMemo(() => {
    const available: MoleListAppsEntry[] = []
    const upToDate: MoleListAppsEntry[] = []
    for (const app of mechanismApps) {
      const r = checkResults.get(app.path)
      // latest 为空（Electron / 网络失败）→ 行静默消失（不进任何已检查分区）
      if (!r?.latest_version) continue
      if (isVersionNewer(r.latest_version, app.version) && osIsInstallable(r.minimum_os, runningOs)) {
        available.push(app)
      } else {
        upToDate.push(app)
      }
    }
    const byName = (a: MoleListAppsEntry, b: MoleListAppsEntry) =>
      a.name.localeCompare(b.name)
    available.sort(byName)
    upToDate.sort(byName)
    return { available, upToDate }
  }, [mechanismApps, checkResults, runningOs])

  // ── 手动检查（对齐 checkNow：并发 6 由后端保证；完成后 brew 直接覆盖）──
  const handleCheck = () => {
    if (checking) return
    // 原地安装进行中（下载/验证/替换）不允许重新检查：避免与安装会话交错
    if (
      [...installs.values()].some(
        (v) => v.stage === 'downloading' || v.stage === 'verifying' || v.stage === 'installing'
      )
    ) {
      return
    }
    setChecking(true)
    tauri
      .mole_updates_check({ app_paths: mechanismApps.map((a) => a.path) })
      .then((res) => {
        setRunningOs(res.running_os ?? '')
        const map = new Map<string, AppCheckResult>()
        for (const r of res.apps ?? []) map.set(r.path, r)
        setCheckResults(map)
        setBrewItems(res.brew ?? [])
        setChecked(true)
        // 重新检查后重置交接态（版本已刷新，旧交接标记失效）
        setHandedOff(new Map())
        // 已完成的安装标记同样过期（版本信息已刷新）；进行中的会话保留
        setInstalls((prev) => {
          const next = new Map<string, InstallUi>()
          for (const [k, v] of prev) {
            if (v.stage !== 'completed') next.set(k, v)
          }
          return next
        })
      })
      .catch(() => {})
      .finally(() => setChecking(false))
  }

  // ── 深链更新（对齐 Burrow `.handedOff`：打开成功 → 行内标记「已在 X 中继续」，
  //     按钮换为状态文字防重复点击；失败不再静默——mole_updates_apply 已回传
  //     open 的真实退出码，弹错让用户知道原因）──
  const handleUpdate = (app: MoleListAppsEntry) => {
    const dest: 'app' | 'appstore' = app.update_source === 'app_store' ? 'appstore' : 'app'
    const applied = () => setHandedOff((prev) => new Map(prev).set(app.path, dest))
    const failed = (err: unknown) => {
      console.warn('[Updates] apply failed', err)
      moleMessage.error(t('uninstall.updates.applyFailed'))
    }
    if (app.update_source === 'app_store') {
      const r = checkResults.get(app.path)
      if (r?.page_url) {
        tauri.mole_updates_apply({ action: 'open_url', target: r.page_url }).then(applied).catch(failed)
      } else {
        tauri.mole_updates_apply({ action: 'macappstore' }).then(applied).catch(failed)
      }
    } else {
      tauri.mole_updates_apply({ action: 'open_app', target: app.path }).then(applied).catch(failed)
    }
  }

  // ── 原地安装（更新执行引擎）：sparkle 源走 prepare → commit 全流程 ──
  // 语义对齐 Burrow：确认后应用内完成下载/验证；就绪后用户再点「安装并重启」。
  // prepare 失败且原因属「不支持原地更新」时回退深链交接（Burrow 同款降级）。

  const startInstall = async (app: MoleListAppsEntry) => {
    const name = app.display_name || app.name
    const ok = await moleNativeConfirm(t('uninstall.updates.installConfirmTitle'), {
      informativeText: t('uninstall.updates.installConfirmMessage', { name }),
      okLabel: t('uninstall.updates.installConfirmOk'),
    })
    if (!ok) return
    setInstalls((prev) => new Map(prev).set(app.path, { stage: 'downloading', bytes: 0 }))
    try {
      const res = (await tauri.mole_updates_install({
        app_path: app.path,
        current_version: app.version,
      })) as InstallPrepareResult
      setInstalls((prev) => {
        const next = new Map(prev)
        const cur = next.get(app.path)
        next.set(app.path, {
          stage: 'ready',
          bytes: cur?.bytes ?? 0,
          newVersion: res.new_version,
        })
        return next
      })
    } catch (err) {
      console.warn('[Updates] install prepare failed', err)
      setInstalls((prev) => {
        const next = new Map(prev)
        next.delete(app.path)
        return next
      })
      const msg = String(err)
      // 智能回退：缺 SUPublicEDKey / feed / 可选包 = 不支持原地更新 → 打开应用交接
      if (
        msg.includes('SUPublicEDKey') ||
        msg.includes('SUFeedURL') ||
        msg.includes('没有可用的全量更新包')
      ) {
        moleMessage.info(t('uninstall.updates.installFallback'))
        handleUpdate(app)
      } else {
        moleMessage.error(`${t('uninstall.updates.installFailed')}: ${msg}`)
      }
    }
  }

  const commitInstall = async (app: MoleListAppsEntry) => {
    setInstalls((prev) => {
      const next = new Map(prev)
      const cur = next.get(app.path)
      next.set(app.path, {
        stage: 'installing',
        bytes: cur?.bytes ?? 0,
        newVersion: cur?.newVersion,
      })
      return next
    })
    try {
      await tauri.mole_updates_install_commit({ app_path: app.path })
      // completed 由事件补齐（含 newVersion）
    } catch (err) {
      console.warn('[Updates] install commit failed', err)
      moleMessage.error(`${t('uninstall.updates.installFailed')}: ${String(err)}`)
      // 失败回 ready（后端暂存保留，可重试）
      setInstalls((prev) => {
        const next = new Map(prev)
        const cur = next.get(app.path)
        if (cur) {
          next.set(app.path, {
            stage: 'ready',
            bytes: cur.bytes,
            newVersion: cur.newVersion,
          })
        }
        return next
      })
    }
  }

  /** 刷新 brew 行（对齐 upgrade 完成后的 `brewItems = await brewOutdated()`） */
  const refreshBrew = () =>
    tauri
      .mole_updates_brew_outdated()
      .then((items: BrewOutdatedItem[]) => setBrewItems(items ?? []))
      .catch(() => {})

  const handleUpgrade = (item: BrewOutdatedItem) => {
    const id = `${item.kind}:${item.name}`
    if (upgrading.has(id)) return
    setUpgrading((prev) => new Set(prev).add(id))
    tauri
      .mole_updates_brew_upgrade({ name: item.name })
      .then(() => {
        setBrewPhrase('')
        return refreshBrew()
      })
      // 升级结束（无论成败）必刷新 brew 行
      .catch(() => {
        setBrewPhrase('')
        return refreshBrew()
      })
      .finally(() =>
        setUpgrading((prev) => {
          const next = new Set(prev)
          next.delete(id)
          return next
        })
      )
  }

  const handleUpgradeAll = () => {
    if (upgrading.size > 0) return
    const ids = new Set(brewItems.map((b) => `${b.kind}:${b.name}`))
    setUpgrading(ids)
    tauri
      .mole_updates_brew_upgrade({ name: null })
      .then(() => {
        setBrewPhrase('')
        return refreshBrew()
      })
      // 升级结束（无论成败）必刷新 brew 行
      .catch(() => {
        setBrewPhrase('')
        return refreshBrew()
      })
      .finally(() => setUpgrading(new Set()))
  }

  const totalUpdates = available.length + brewItems.length

  /**
   * available 行操作区（优先级）：原地安装状态 > 交接状态 > 「更新」按钮。
   * - sparkle 源点「更新」→ 原地安装流程（确认 → 下载 → 验证 → 安装并重启）；
   * - 其余源（Electron / App Store）维持深链交接。
   */
  const renderRowAction = (app: MoleListAppsEntry) => {
    const inst = installs.get(app.path)
    if (inst) {
      switch (inst.stage) {
        case 'downloading':
          return (
            <span className="text-[10px] text-white/60 whitespace-nowrap">
              {t('uninstall.updates.downloading', { size: formatSize(inst.bytes) })}
            </span>
          )
        case 'verifying':
          return (
            <span className="text-[10px] text-white/60 whitespace-nowrap">
              {t('uninstall.updates.verifying')}
            </span>
          )
        case 'ready':
          return (
            <button
              onClick={() => void commitInstall(app)}
              className="apps-primary-btn text-[11px] font-semibold px-3 py-1 rounded-full"
            >
              {t('uninstall.updates.installAndRestart')}
            </button>
          )
        case 'installing':
          return (
            <span className="text-[10px] text-amber-400 whitespace-nowrap">
              {t('uninstall.updates.installing')}
            </span>
          )
        case 'completed':
          return (
            <span className="text-[10px] text-green-400 whitespace-nowrap">
              {t('uninstall.updates.installed', { version: inst.newVersion ?? '' })}
            </span>
          )
      }
    }
    const handoff = handedOff.get(app.path)
    if (handoff) {
      // 交接完成态（Burrow 同款 amber 语义）：替代按钮，防重复点击
      return (
        <span className="text-[10px] text-amber-400 whitespace-nowrap">
          {t(
            handoff === 'appstore'
              ? 'uninstall.updates.handedOffAppStore'
              : 'uninstall.updates.handedOffApp'
          )}
        </span>
      )
    }
    return (
      <button
        onClick={() => (app.update_source === 'sparkle' ? void startInstall(app) : handleUpdate(app))}
        className="apps-primary-btn text-[11px] font-semibold px-3 py-1 rounded-full"
      >
        {t('uninstall.updates.update')}
      </button>
    )
  }

  return (
    <div className="h-full flex flex-col">
      {/* header（mx-24 补偿根容器移除的留白，px-52 维持原视觉缩进） */}
      <div className="flex items-center gap-2 mr-[24px] px-[24px] py-2 shrink-0">
        {checked || brewItems.length > 0 ? (
          <span className="text-xs text-white/85">
            <strong className="text-[var(--text-primary)]">{totalUpdates}</strong>{' '}
            {totalUpdates === 1 ? t('uninstall.updates.count') : t('uninstall.updates.countAvailable')}
          </span>
        ) : (
          <span className="text-[10px] text-white/60">
            {t('uninstall.updates.autoHint')}
          </span>
        )}
        <div className="flex-1" />
        {checking || brewSurfacing ? (
          <span className="text-[10px] text-white/60">
            {checking ? t('uninstall.updates.checking') : t('uninstall.updates.checkingBrew')}
          </span>
        ) : null}
        <button
          onClick={handleCheck}
          disabled={checking}
          className={
            checked
              ? 'apps-ghost-btn text-[11px] font-semibold px-3 py-1 rounded-full disabled:opacity-50'
              : 'apps-primary-btn text-[11px] font-semibold px-3 py-1 rounded-full disabled:opacity-50'
          }
        >
          {checked ? t('uninstall.updates.recheck') : t('uninstall.updates.check')}
        </button>
        {checked && brewItems.length > 0 && (
          <button
            onClick={handleUpgradeAll}
            disabled={upgrading.size > 0}
            className="apps-ghost-btn text-[11px] font-semibold px-3 py-1 rounded-full disabled:opacity-50"
          >
            {upgrading.size > 0 ? t('uninstall.updates.upgrading') : t('uninstall.updates.upgradeAllBrew')}
          </button>
        )}
      </div>

      {/* hairline（mx-24 补偿根容器移除的留白） */}
      <div className="shrink-0 h-px bg-white/[0.12] mx-[24px]" />

      {/* 列表（mole-scroll 贴窗口边缘，px-24 wrapper 补偿留白） */}
      <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
        <div className="px-[24px] pb-2">
        {/* 空态：checked 后无任何更新（对齐 "Everything's up to date"） */}
        {checked && available.length === 0 && brewItems.length === 0 && (
          <div className="flex flex-col items-center gap-2.5 pt-10">
            <CheckCircle2 size={30} className="text-green-400" />
            <span className="text-[15px] font-medium text-[var(--text-primary)]">
              {t('uninstall.updates.allUpToDate')}
            </span>
          </div>
        )}

        {/* Updates available：app 行 + brew 行 */}
        {(available.length > 0 || brewItems.length > 0) && (
          <>
            <SectionHeader title={t('uninstall.updates.section.available')} count={available.length + brewItems.length} />
            {available.map((app) => {
              const r = checkResults.get(app.path)
              const latest = r?.latest_version ?? undefined
              const isNewer = latest ? isVersionNewer(latest, app.version) : false
              return (
                <AppUpdateRow
                  key={app.path}
                  app={app}
                  latestVersion={isNewer ? latest : undefined}
                  action={renderRowAction(app)}
                />
              )
            })}
            {brewItems.map((item) => {
              const id = `${item.kind}:${item.name}`
              const isUpgrading = upgrading.has(id)
              return (
                <div key={id} className="flex items-center gap-3 px-[24px] py-2">
                  <div className="w-7 shrink-0 flex items-center justify-center">
                    {item.kind === 'cask' ? (
                      <AppWindow size={14} className="text-[var(--accent)]" />
                    ) : (
                      <Package size={14} className="text-[var(--accent)]" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
                        {item.name}
                      </span>
                      {sourceChip('homebrew')}
                    </div>
                    <div className="text-[10px] font-mono text-white/60 truncate">
                      {isUpgrading && brewPhrase ? (
                        <span className="text-[var(--accent)]">{brewPhrase}</span>
                      ) : (
                        `${item.installed} → ${item.latest}`
                      )}
                    </div>
                  </div>
                  <div className="shrink-0 w-[64px] flex justify-end">
                    {isUpgrading ? (
                      <span className="text-[10px] text-white/60">{t('uninstall.updates.upgrading')}</span>
                    ) : (
                      <button
                        onClick={() => handleUpgrade(item)}
                        className="apps-primary-btn text-[11px] font-semibold px-3 py-1 rounded-full"
                      >
                        {t('uninstall.updates.update')}
                      </button>
                    )}
                  </div>
                </div>
              )
            })}
          </>
        )}

        {/* Up to date（仅 checked 后） */}
        {checked && upToDate.length > 0 && (
          <>
            <SectionHeader title={t('uninstall.updates.section.upToDate')} count={upToDate.length} />
            {upToDate.map((app) => (
              <AppUpdateRow key={app.path} app={app} />
            ))}
          </>
        )}

        {/* 未 checked：有更新机制的 app 全部展示（对齐 "Apps with an update mechanism"） */}
        {!checked && mechanismApps.length > 0 && (
          <>
            <SectionHeader title={t('uninstall.updates.section.mechanism')} count={mechanismApps.length} />
            {[...mechanismApps]
              .sort((a, b) => a.name.localeCompare(b.name))
              .map((app) => (
                <AppUpdateRow key={app.path} app={app} />
              ))}
          </>
        )}

        {/* Not checkable：永远显示在最后（对齐 uncheckableApps） */}
        {uncheckableApps.length > 0 && (
          <>
            <SectionHeader title={t('uninstall.updates.section.uncheckable')} count={uncheckableApps.length} />
            <p className="px-[24px] pb-1 text-[10px] font-mono text-white/60">
              {t('uninstall.updates.uncheckableHint')}
            </p>
            {[...uncheckableApps]
              .sort((a, b) => a.name.localeCompare(b.name))
              .map((app) => (
                <AppUpdateRow key={app.path} app={app} compact />
              ))}
          </>
          )}
        </div>
      </SimpleBar>
    </div>
  )
}
