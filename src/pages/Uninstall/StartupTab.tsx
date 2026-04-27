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
import { iconService } from '@/utils/iconService'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { SectionHeader } from '@/components/business/Apps/SectionHeader'
import { Badge } from '@/components/business/Apps/Badge'
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

const STATUS_CONFIG: Record<ServiceStatus, { label: string; color: string; bg: string }> = {
  running: { label: '运行中', color: '#4ade80', bg: 'rgba(74,222,128,0.12)' },
  scheduled: { label: '定时', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  stopped: { label: '已停止', color: '#94a3b8', bg: 'rgba(148,163,184,0.10)' },
  failed: { label: '失败', color: '#f87171', bg: 'rgba(248,113,113,0.12)' },
  unloaded: { label: '未加载', color: '#94a3b8', bg: 'rgba(148,163,184,0.08)' },
  disabled: { label: '已禁用', color: '#fbbf24', bg: 'rgba(251,191,36,0.10)' },
  unknown: { label: '未知', color: '#94a3b8', bg: 'rgba(148,163,184,0.08)' },
}

const PROVENANCE_LABEL: Record<Provenance, string> = {
  homebrew: 'Brew',
  user_plist: '用户',
  vendor_app: '厂商',
  system: '系统',
  runtime_only: '运行时',
  unknown: '',
}

const ENABLE_STATUS_CONFIG: Record<EnableStatus, { label: string; color: string; bg: string }> = {
  all_enabled: { label: '全部开启', color: '#4ade80', bg: 'rgba(74,222,128,0.12)' },
  some_enabled: { label: '部分开启', color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  all_disabled: { label: '全部关闭', color: '#94a3b8', bg: 'rgba(148,163,184,0.10)' },
}

// ── 辅助函数 ──

function statusChip(status: ServiceStatus) {
  const cfg = STATUS_CONFIG[status]
  return <Badge label={cfg.label} color={cfg.color} bg={cfg.bg} />
}

function provenanceTag(origin: Provenance) {
  const label = PROVENANCE_LABEL[origin]
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
      moleMessage.error('启动项扫描失败')
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
      .mole_request_admin_session({ prompt: '显示登录项需要管理员权限' })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.error('管理员认证失败，无法扫描登录项')
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
        moleMessage.error(res.message || '操作失败')
      }
      // 操作后重扫
      await reload(loginItemsIncluded)
    } catch (err) {
      console.error('[Startup] action failed', err)
      moleMessage.error('操作执行失败')
    }
  }

  /** App 级一键开关 */
  const toggleAppGroup = (group: AppGroup, enable: boolean) => {
    const action = enable ? 'enable' : 'disable'
    const actionable = group.services.filter(isActionable)
    Promise.all(actionable.map((s) => tauri.mole_startup_action({ service_id: s.id, action })))
      .then(() => reload(loginItemsIncluded))
      .catch(() => moleMessage.error('批量操作失败'))
  }

  /** 预加载 App 图标 */
  useEffect(() => {
    if (!inventory) return
    const paths = new Set<string>()
    for (const g of inventory.app_groups) {
      if (g.app_path) paths.add(g.app_path)
    }
    paths.add(SYSTEM_PLIST_ICON_PATH)
    iconService.preloadIcons([...paths]).catch(() => {})
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
              现代登录项（Login Items）未显示
            </div>
            <div className="text-[11px] text-white/60">
              扫描需要管理员授权，将弹出系统认证框（一次授权后不再重复）
            </div>
          </div>
          <button
            onClick={scanLoginItems}
            disabled={scanningLoginItems}
            className="text-[11px] font-semibold text-[var(--text-primary)] px-3 py-1.5 rounded-full bg-white/[0.10] border border-white/[0.12] hover:bg-white/[0.16] disabled:opacity-50 shrink-0"
          >
            {scanningLoginItems ? '扫描中…' : '扫描登录项'}
          </button>
          <button
            onClick={() => setBannerDismissed(true)}
            title="关闭"
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
          <div className="text-xs">正在扫描启动项…</div>
        </div>
      ) : (
        <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
          <div className="py-1">
            {filteredGroups.length === 0 && filteredStandalone.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full text-white/60 py-20">
                <div className="text-sm">未找到匹配的启动项</div>
              </div>
            ) : (
              <>
                {/* 应用启动项 */}
                {filteredGroups.length > 0 && (
                  <>
                    <SectionHeader title="应用启动项" count={filteredGroups.length} />
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
                    <SectionHeader title="后台服务" count={filteredStandalone.length} />
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
  const statusCfg = ENABLE_STATUS_CONFIG[group.enable_status]
  const allOn = group.enable_status !== 'all_disabled'
  const actionable = group.services.filter(isActionable)

  // App 图标
  const icon = iconService.getCachedSync(group.app_path)

  return (
    <div>
      {/* App 行 */}
      <div
        className="flex items-center gap-2.5 px-[24px] py-2.5 rounded-lg hover:bg-black/[0.25] transition-colors cursor-pointer"
        onClick={onToggleExpand}
      >
        {icon ? (
          <img src={icon} alt={group.app_name} className="w-7 h-7 rounded-md object-contain shrink-0" />
        ) : (
          <AppIcon name={group.app_name} path={group.app_path} />
        )}

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
              {group.app_name}
            </span>
            <Badge label={statusCfg.label} color={statusCfg.color} bg={statusCfg.bg} />
            <span className="text-[10px] text-white/45 font-mono">{group.services.length} 项</span>
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
  const actionable = isActionable(service)
  const enabled = service.enabled !== false
  const hasHealth = service.health.length > 0

  // 系统 plist 图标
  const sysIcon = iconService.getCachedSync(SYSTEM_PLIST_ICON_PATH)

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
          {statusChip(service.status)}
          {provenanceTag(service.origin.kind)}
          {hasHealth && (
            <Badge
              label="问题"
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
              title="停止服务"
              className="text-white/40 hover:text-red-400 p-1"
            >
              <Square size={11} />
            </button>
          )}
          {(service.status === 'stopped' || service.status === 'failed') && service.loaded && (
            <button
              onClick={() => onAction(service.id, 'start')}
              title="启动服务"
              className="text-white/40 hover:text-green-400 p-1"
            >
              <Play size={11} />
            </button>
          )}

          {/* 启用/禁用开关 */}
          <button
            role="switch"
            aria-checked={enabled}
            title={enabled ? '禁用此启动项' : '启用此启动项'}
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
        <span title="仅查看——由系统或所属应用管理" className="text-white/35 shrink-0">
          <Lock size={11} />
        </span>
      )}
    </div>
  )
}
