import { useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  FolderOpen,
  Loader2,
  Lock,
  Play,
  ShieldAlert,
  Square,
} from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import useTauri from '@/hooks/useTauri'
import { moleMessage } from '@/components/ui'
import { nativeIconRegistry } from '@/utils/nativeIconRegistry'
import { useNativeIcon } from '@/hooks/useNativeIcon'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { SectionHeader } from '@/components/business/Apps/SectionHeader'
import { Badge } from '@/components/business/Apps/Badge'
import { useI18n } from '@/i18n'
import type { TFunction, TranslationKey } from '@/i18n'
import type {
  AppGroup,
  EnableStatus,
  Provenance,
  ServiceStatus,
  StartupFilter,
  StartupInventory,
  StartupService,
} from '@/types/mole'

// ── 状态样式映射 ──

const STATUS_CONFIG: Record<ServiceStatus, { labelKey: TranslationKey; color: string; bg: string }> = {
  running: { labelKey: 'uninstall.startup.status.running', color: '#4ade80', bg: 'rgba(74,222,128,0.12)' },
  scheduled: { labelKey: 'uninstall.startup.status.scheduled', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  stopped: { labelKey: 'uninstall.startup.status.stopped', color: '#94a3b8', bg: 'rgba(148,163,184,0.10)' },
  failed: { labelKey: 'uninstall.startup.status.failed', color: '#f87171', bg: 'rgba(248,113,113,0.12)' },
  unloaded: { labelKey: 'uninstall.startup.status.unloaded', color: '#94a3b8', bg: 'rgba(148,163,184,0.08)' },
  disabled: { labelKey: 'uninstall.startup.status.disabled', color: '#fbbf24', bg: 'rgba(251,191,36,0.10)' },
  unknown: { labelKey: 'uninstall.startup.status.unknown', color: '#94a3b8', bg: 'rgba(148,163,184,0.08)' },
}

const PROVENANCE_LABEL: Record<Provenance, TranslationKey> = {
  homebrew: 'uninstall.startup.provenance.homebrew',
  user_plist: 'uninstall.startup.provenance.user',
  vendor_app: 'uninstall.startup.provenance.vendor',
  system: 'uninstall.startup.provenance.system',
  runtime_only: 'uninstall.startup.provenance.runtime',
  unknown: 'uninstall.startup.provenance.unknown',
}

const ENABLE_STATUS_CONFIG: Record<EnableStatus, { labelKey: TranslationKey; color: string; bg: string }> = {
  all_enabled: { labelKey: 'uninstall.startup.enable.allEnabled', color: '#4ade80', bg: 'rgba(74,222,128,0.12)' },
  some_enabled: { labelKey: 'uninstall.startup.enable.someEnabled', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  all_disabled: { labelKey: 'uninstall.startup.enable.allDisabled', color: '#94a3b8', bg: 'rgba(148,163,184,0.10)' },
}

// ── 辅助函数 ──

function statusChip(status: ServiceStatus, t: TFunction) {
  const cfg = STATUS_CONFIG[status]
  return <Badge label={t(cfg.labelKey)} color={cfg.color} bg={cfg.bg} />
}

function provenanceTag(origin: Provenance, t: TFunction) {
  const label = t(PROVENANCE_LABEL[origin])
  if (!label) return null
  return (
    <span className="text-[9px] px-1.5 py-0.5 rounded bg-white/[0.06] text-white/50 font-mono">
      {label}
    </span>
  )
}

/** 是否可操作（非只读、非保护） */
function isActionable(svc: StartupService): boolean {
  return svc.safety_level !== 'readonly_system' && svc.safety_level !== 'protected_vendor'
}

/** 系统 plist 图标采样路径 */
const SYSTEM_PLIST_ICON_PATH = '/System/Library/CoreServices/SystemVersion.plist'

// ── 主组件 ──

interface StartupTabProps {
  filter: StartupFilter
  searchText: string
  reloadTick: number
}

export function StartupTab({ filter, searchText, reloadTick }: StartupTabProps) {
  const tauri = useTauri()
  const { t } = useI18n()

  const [inventory, setInventory] = useState<StartupInventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [loginItemsIncluded, setLoginItemsIncluded] = useState(false)
  const [scanningLoginItems, setScanningLoginItems] = useState(false)
  const [bannerDismissed, setBannerDismissed] = useState(false)
  const [expandedApps, setExpandedApps] = useState<Set<string>>(new Set())
  const started = useRef(false)
  const lastTick = useRef(0)

  const reload = async (includeLoginItems: boolean) => {
    setLoading(true)
    try {
      const res = await tauri.mole_startup_scan({
        include_login_items: includeLoginItems,
        show_system: false,
      })
      setInventory(res as StartupInventory)
      if (includeLoginItems) setLoginItemsIncluded(true)
    } catch (err) {
      console.error('[Startup] scan failed', err)
      moleMessage.error(t('uninstall.startup.scanFailed'))
    } finally {
      setLoading(false)
      setScanningLoginItems(false)
    }
  }

  useEffect(() => {
    if (started.current) return
    started.current = true
    reload(false)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (reloadTick > 0 && reloadTick !== lastTick.current) {
      lastTick.current = reloadTick
      reload(loginItemsIncluded)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reloadTick])

  /** 扫描登录项（需管理员授权） */
  const scanLoginItems = async () => {
    setScanningLoginItems(true)
    const auth = (await tauri
      .mole_request_admin_session({ prompt: t('uninstall.startup.loginAuthPrompt') })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.error(t('uninstall.startup.loginAuthFailed'))
      }
      setScanningLoginItems(false)
      return
    }
    await reload(true)
  }

  /** 执行操作 */
  const doAction = async (serviceId: string, action: string) => {
    try {
      const res = await tauri.mole_startup_action({ service_id: serviceId, action })
      if (!res.success) {
        moleMessage.error(res.message || t('uninstall.startup.actionFailed'))
      }
      // 操作后重扫
      await reload(loginItemsIncluded)
    } catch (err) {
      console.error('[Startup] action failed', err)
      moleMessage.error(t('uninstall.startup.actionRunFailed'))
    }
  }

  /** App 级一键开关 */
  const toggleAppGroup = (group: AppGroup, enable: boolean) => {
    const action = enable ? 'enable' : 'disable'
    const actionable = group.services.filter(isActionable)
    Promise.all(actionable.map((s) => tauri.mole_startup_action({ service_id: s.id, action })))
      .then(() => reload(loginItemsIncluded))
      .catch(() => moleMessage.error(t('uninstall.startup.batchFailed')))
  }

  /** 预加载 App 图标 */
  useEffect(() => {
    if (!inventory) return
    const paths = new Set<string>()
    for (const g of inventory.app_groups) {
      if (g.app_path) paths.add(g.app_path)
    }
    paths.add(SYSTEM_PLIST_ICON_PATH)
    nativeIconRegistry.resolveIdle([...paths]).catch(() => {})
  }, [inventory])

  /** 切换 App 展开 */
  const toggleExpand = (key: string) => {
    setExpandedApps((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  // ── 筛选 ──

  const filteredGroups = useMemo(() => {
    if (!inventory) return []
    let groups = inventory.app_groups
    const q = searchText.trim().toLowerCase()

    if (filter === 'services') return [] // "服务"筛选只显示 standalone
    if (filter === 'problems') {
      groups = groups
        .map((g) => ({ ...g, services: g.services.filter((s) => s.health.length > 0) }))
        .filter((g) => g.services.length > 0)
    }
    if (q) {
      groups = groups
        .map((g) => ({
          ...g,
          services: g.services.filter(
            (s) =>
              s.label.toLowerCase().includes(q) ||
              s.display_name.toLowerCase().includes(q) ||
              g.app_name.toLowerCase().includes(q),
          ),
        }))
        .filter((g) => g.services.length > 0 || g.app_name.toLowerCase().includes(q))
    }
    return groups
  }, [inventory, filter, searchText])

  const filteredStandalone = useMemo(() => {
    if (!inventory) return []
    let items = inventory.standalone_services
    const q = searchText.trim().toLowerCase()

    if (filter === 'apps') return [] // "应用"筛选只显示 app_groups
    if (filter === 'problems') {
      items = items.filter((s) => s.health.length > 0)
    }
    if (q) {
      items = items.filter(
        (s) =>
          s.label.toLowerCase().includes(q) ||
          s.display_name.toLowerCase().includes(q) ||
          (s.brew_formula ?? '').toLowerCase().includes(q),
      )
    }
    return items
  }, [inventory, filter, searchText])

  // ── 渲染 ──

  return (
    <div className="h-full flex flex-col">
      {/* 登录项授权 Banner */}
      {!loginItemsIncluded && !bannerDismissed && (
        <div className="mx-[76px] mt-2.5 shrink-0 flex items-center gap-2.5 px-3 py-2 rounded-xl border border-amber-400/25 bg-[#1a1812]/[0.96]">
          <ShieldAlert size={15} className="text-amber-400 shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="text-[12px] font-semibold text-[var(--text-primary)]">
              {t('uninstall.startup.banner.title')}
            </div>
            <div className="text-[11px] text-white/60">
              {t('uninstall.startup.banner.desc')}
            </div>
          </div>
          <button
            onClick={scanLoginItems}
            disabled={scanningLoginItems}
            className="text-[11px] font-semibold text-[var(--text-primary)] px-3 py-1.5 rounded-full bg-white/[0.10] border border-white/[0.12] hover:bg-white/[0.16] disabled:opacity-50 shrink-0"
          >
            {scanningLoginItems ? t('uninstall.startup.scanning') : t('uninstall.startup.scanLoginItems')}
          </button>
          <button
            onClick={() => setBannerDismissed(true)}
            title={t('common.close')}
            className="text-white/40 hover:text-white/70 shrink-0"
          >
            <span className="text-[11px] font-bold px-0.5">✕</span>
          </button>
        </div>
      )}

      {/* 列表 */}
      {loading && !inventory ? (
        <div className="flex-1 flex flex-col items-center justify-center gap-2 text-white/55">
          <Loader2 size={16} className="animate-spin" />
          <div className="text-xs">{t('uninstall.startup.scanningList')}</div>
        </div>
      ) : (
        <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
          <div className="py-1">
            {filteredGroups.length === 0 && filteredStandalone.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full text-white/60 py-20">
                <div className="text-sm">{t('uninstall.startup.noMatch')}</div>
              </div>
            ) : (
              <>
                {/* 应用启动项 */}
                {filteredGroups.length > 0 && (
                  <>
                    <SectionHeader title={t('uninstall.startup.section.apps')} count={filteredGroups.length} />
                    <div className="space-y-0.5">
                      {filteredGroups.map((group) => (
                        <AppGroupRow
                          key={group.bundle_id || group.app_path}
                          group={group}
                          expanded={expandedApps.has(group.bundle_id || group.app_path)}
                          onToggleExpand={() => toggleExpand(group.bundle_id || group.app_path)}
                          onAction={doAction}
                          onBatchToggle={toggleAppGroup}
                        />
                      ))}
                    </div>
                  </>
                )}

                {/* 后台服务 */}
                {filteredStandalone.length > 0 && (
                  <>
                    <SectionHeader title={t('uninstall.startup.section.services')} count={filteredStandalone.length} />
                    <div className="space-y-0.5">
                      {filteredStandalone.map((svc) => (
                        <ServiceRow key={svc.id} service={svc} onAction={doAction} />
                      ))}
                    </div>
                  </>
                )}
              </>
            )}
          </div>
        </SimpleBar>
      )}
    </div>
  )
}

// ── App 分组行 ──

function AppGroupRow({
  group,
  expanded,
  onToggleExpand,
  onAction,
  onBatchToggle,
}: {
  group: AppGroup
  expanded: boolean
  onToggleExpand: () => void
  onAction: (id: string, action: string) => void
  onBatchToggle: (group: AppGroup, enable: boolean) => void
}) {
  const { t } = useI18n()
  const statusCfg = ENABLE_STATUS_CONFIG[group.enable_status]
  const allOn = group.enable_status !== 'all_disabled'
  const actionable = group.services.filter(isActionable)

  return (
    <div>
      {/* App 行 */}
      <div
        className="flex items-center gap-2.5 px-[24px] py-2.5 rounded-lg hover:bg-black/[0.25] transition-colors cursor-pointer"
        onClick={onToggleExpand}
      >
        <AppIcon name={group.app_name} path={group.app_path} />

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
              {group.app_name}
            </span>
            <Badge label={t(statusCfg.labelKey)} color={statusCfg.color} bg={statusCfg.bg} />
            <span className="text-[10px] text-white/45 font-mono">{t('uninstall.startup.itemCount', { count: group.services.length })}</span>
          </div>
        </div>

        {/* 展开箭头 */}
        <span className="text-white/40 shrink-0">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>

        {/* App 级一键开关 */}
        {actionable.length > 0 && (
          <button
            role="switch"
            aria-checked={allOn}
            onClick={(e) => {
              e.stopPropagation()
              onBatchToggle(group, !allOn)
            }}
            className="relative w-8 h-[18px] rounded-full transition-colors shrink-0"
            style={{ background: allOn ? 'rgba(99,102,241,0.9)' : 'rgba(255,255,255,0.16)' }}
          >
            <span
              className="absolute top-[2px] w-[14px] h-[14px] rounded-full bg-white transition-[left] duration-150"
              style={{ left: allOn ? 16 : 2 }}
            />
          </button>
        )}
      </div>

      {/* 展开的子项 */}
      {expanded && (
        <div className="ml-[52px] border-l border-white/[0.06] pl-3">
          {group.services.map((svc) => (
            <ServiceRow key={svc.id} service={svc} onAction={onAction} compact />
          ))}
        </div>
      )}
    </div>
  )
}

// ── 服务行 ──

function ServiceRow({
  service,
  onAction,
  compact,
}: {
  service: StartupService
  onAction: (id: string, action: string) => void
  compact?: boolean
}) {
  const { t } = useI18n()
  const actionable = isActionable(service)
  const enabled = service.enabled !== false
  const hasHealth = service.health.length > 0

  // 系统 plist 图标（注册表订阅，解析完成自动刷新）
  const sysIcon = useNativeIcon(SYSTEM_PLIST_ICON_PATH)

  return (
    <div
      className={`flex items-center gap-2 rounded-lg hover:bg-black/[0.20] transition-colors ${
        compact ? 'px-3 py-1.5' : 'px-[24px] py-2'
      }`}
    >
      {!compact && (
        <span className="shrink-0">
          {sysIcon ? (
            <img src={sysIcon} alt="" className="w-6 h-6 object-contain" />
          ) : (
            <FolderOpen size={16} className="text-white/40" />
          )}
        </span>
      )}

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className={`font-medium text-[var(--text-primary)] truncate ${compact ? 'text-[11px]' : 'text-[12px]'}`}>
            {service.display_name}
          </span>
          {statusChip(service.status, t)}
          {provenanceTag(service.origin.kind, t)}
          {hasHealth && (
            <Badge
              label={t('uninstall.startup.issue')}
              color="#ef4444"
              bg="rgba(239,68,68,0.12)"
              icon={<AlertTriangle size={9} className="inline -mt-px" />}
              title={service.health.join('; ')}
            />
          )}
        </div>
        {!compact && (
          <div className="text-[10px] font-mono text-white/45 truncate mt-0.5">
            {service.label}
            {service.brew_formula && <span className="text-emerald-400/70 ml-1.5">brew: {service.brew_formula}</span>}
          </div>
        )}
      </div>

      {/* 操作按钮 */}
      {actionable ? (
        <div className="flex items-center gap-1 shrink-0">
          {/* 停止/启动按钮 */}
          {service.status === 'running' && (
            <button
              onClick={() => onAction(service.id, 'stop')}
              title={t('uninstall.startup.stopService')}
              className="text-white/40 hover:text-red-400 p-1"
            >
              <Square size={11} />
            </button>
          )}
          {(service.status === 'stopped' || service.status === 'failed') && service.loaded && (
            <button
              onClick={() => onAction(service.id, 'start')}
              title={t('uninstall.startup.startService')}
              className="text-white/40 hover:text-green-400 p-1"
            >
              <Play size={11} />
            </button>
          )}

          {/* 启用/禁用开关 */}
          <button
            role="switch"
            aria-checked={enabled}
            title={enabled ? t('uninstall.startup.disableItem') : t('uninstall.startup.enableItem')}
            onClick={() => onAction(service.id, enabled ? 'disable' : 'enable')}
            className="relative w-7 h-[16px] rounded-full transition-colors"
            style={{ background: enabled ? 'rgba(99,102,241,0.9)' : 'rgba(255,255,255,0.16)' }}
          >
            <span
              className="absolute top-[2px] w-[12px] h-[12px] rounded-full bg-white transition-[left] duration-150"
              style={{ left: enabled ? 14 : 2 }}
            />
          </button>
        </div>
      ) : (
        <span title={t('uninstall.startup.readonly')} className="text-white/35 shrink-0">
          <Lock size={11} />
        </span>
      )}
    </div>
  )
}
