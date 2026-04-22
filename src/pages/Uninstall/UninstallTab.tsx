import { useMemo, useState, useCallback } from 'react'
import { ChevronRight, ChevronDown, AlertTriangle, FolderOpen, Eraser, CheckCircle2, XCircle } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { formatSize } from '@/utils/format'
import { useI18n } from '@/i18n'
import useTauri from '@/hooks/useTauri'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { MoleCheckbox, CleaningProgressBar } from '@/components/ui'
import type { CleaningAppState } from '@/hooks/useUninstallProgress'
import type { MoleListAppsEntry, MoleUninstallResult } from '@/types/mole'

// ── 配色（对齐 Clean 页面）──
const ACCENT_GREEN = '#64dfa7' // 强调文字 / 链接

// ── 残留文件分组 ──

interface LeftoverEntry {
  path: string
  size: number
  kind: string
  kindLabel: string
  autoSelected: boolean // true = Auto selected 组；false = Needs review 组
}

const GROUP_LABEL_MAP: Record<string, string> = {
  application_support: 'App Support',
  preferences: 'Preferences',
  container: 'Container',
  helper: 'Helper',
  loginItem: 'Login Item',
  cache: 'Cache',
  logs: 'Logs',
  group_container: 'Group Container',
  saved_state: 'Saved State',
  webkit: 'WebKit',
  http_storage: 'HTTP Storage',
  system_file: 'System File',
  other: 'Other',
}

// 对齐 Burrow UninstallPreview.classify：安全项自动选中，缓存/日志/未知项需审阅
const AUTO_SELECTED_KINDS = new Set([
  'bundle',
  'application_support',
  'preferences',
  'container',
  'helper',
  'loginItem',
])

function buildLeftovers(entry: MoleListAppsEntry, preview: MoleUninstallResult | null): LeftoverEntry[] {
  const items: LeftoverEntry[] = []
  // App Bundle 本身（恒选中）
  items.push({
    path: entry.path,
    size: entry.size_bytes || 0,
    kind: 'bundle',
    kindLabel: 'App Bundle',
    autoSelected: true,
  })
  for (const f of preview?.related_files ?? []) {
    const kind = f.type || 'other'
    items.push({
      path: f.path,
      size: f.size || 0,
      kind,
      kindLabel: GROUP_LABEL_MAP[kind] || kind,
      autoSelected: AUTO_SELECTED_KINDS.has(kind),
    })
  }
  // review-only 系统文件：归入 Needs review 且标记为不可删（只读）
  for (const f of preview?.review_only_files ?? []) {
    const kind = f.type || 'system_file'
    items.push({
      path: f.path,
      size: f.size || 0,
      kind,
      kindLabel: GROUP_LABEL_MAP[kind] || kind,
      autoSelected: false,
    })
  }
  return items
}

// ── 行内展开的残留审阅面板 ──

interface LeftoverPanelProps {
  entry: MoleListAppsEntry
  preview: MoleUninstallResult | null
  leftovers: LeftoverEntry[]
  checkedPaths: Set<string>
  previewLoading: boolean
  cleaningState: CleaningAppState | undefined
  onTogglePath: (path: string) => void
  onToggleGroup: (paths: string[], checked: boolean) => void
  onClearDataOnly: (app: MoleListAppsEntry) => void
  /** 是否为卸载模式（true=卸载，false=清理数据） */
  isUninstallMode?: boolean
}

function LeftoverPanel({
  entry,
  preview,
  leftovers,
  checkedPaths,
  previewLoading,
  cleaningState,
  onTogglePath,
  onToggleGroup,
  onClearDataOnly,
  isUninstallMode = false,
}: LeftoverPanelProps) {
  const tauri = useTauri()
  const { t } = useI18n()
  const displayName = entry.display_name || entry.name
  const prettyPath = entry.path.replace(/^\/Users\/[^/]+/, '~')
  const autoGroup = leftovers.filter((l) => l.autoSelected)
  const reviewGroup = leftovers.filter((l) => !l.autoSelected)

  // 检测 Clear Data 模式是否已激活：bundle 未勾选，但有残留被勾选
  const isDataOnlyMode = !checkedPaths.has(entry.path) && 
    leftovers.some((l) => l.kind !== 'bundle' && l.kind !== 'system_file' && checkedPaths.has(l.path))

  // review-only（system_file 等）不可勾选
  const toggleable = (l: LeftoverEntry) => l.kind !== 'system_file'

  const groupHeader = (title: string, items: LeftoverEntry[]) => {
    const toggleableItems = items.filter(toggleable)
    const selectedCount = toggleableItems.filter((l) => checkedPaths.has(l.path)).length
    const allSelected = selectedCount === toggleableItems.length && toggleableItems.length > 0
    const someSelected = selectedCount > 0 && !allSelected
    return (
      <div className="flex items-center gap-2 px-3 pt-2.5 pb-1 select-none">
        <MoleCheckbox
          checked={allSelected}
          partial={someSelected}
          onClick={() => onToggleGroup(toggleableItems.map((l) => l.path), !allSelected)}
        />
        <span className="text-[10px] font-semibold tracking-wider text-white/60 uppercase">
          {title}
        </span>
        <span className="text-[10px] text-white/60 font-mono">
          {selectedCount}/{toggleableItems.length}
        </span>
      </div>
    )
  }

  const entryRow = (l: LeftoverEntry) => {
    const ticked = checkedPaths.has(l.path)
    const canToggle = toggleable(l)
    const isReviewOnly = l.kind === 'system_file'
    return (
      <div key={l.path} className="flex items-center gap-2.5 px-3 py-1.5 select-none">
        <MoleCheckbox
          checked={ticked}
          disabled={!canToggle}
          onClick={() => onTogglePath(l.path)}
        />
        <span
          className="text-[11px] font-medium text-white/85 w-[92px] shrink-0"
          style={{ color: isReviewOnly ? 'rgba(250,204,21,0.85)' : 'rgba(255,255,255,0.85)' }}
        >
          {l.kindLabel}
        </span>
        <span className="text-[11px] font-mono text-white/60 flex-1 min-w-0 truncate">
          {l.path}
        </span>
        {isReviewOnly && (
          <AlertTriangle size={12} className="text-yellow-400 shrink-0" />
        )}
        <span className="text-[10px] font-mono text-white/60 shrink-0 tabular-nums">
          {l.size > 0 ? formatSize(l.size) : '—'}
        </span>
        <button
          onClick={() => tauri.clean_reveal_in_finder({ path: l.path }).catch(() => {})}
          title={t('common.revealInFinder')}
          className="text-white/60 hover:text-[var(--text-primary)] shrink-0"
        >
          <FolderOpen size={12} />
        </button>
      </div>
    )
  }

  return (
    <div className="mx-[52px] my-2 rounded-xl overflow-hidden border border-white/[0.14] bg-black/40">
      {/* 面板头：名称 + bundle 路径 + 已选计数 + 全选 */}
      <div className="flex items-center gap-2 px-3 pt-2.5">
        <div className="min-w-0 flex-1">
          <div className="text-xs font-semibold text-[var(--text-primary)]">{displayName}</div>
          <div className="text-[10px] font-mono text-white/60 truncate">
            {prettyPath}
          </div>
        </div>
        <span className="text-[10px] font-mono text-white/85 shrink-0">
          {t('uninstall.leftover.selectedOf', {
            selected: leftovers.filter((l) => checkedPaths.has(l.path)).length,
            total: leftovers.length,
          })}
        </span>
        <button
          onClick={() => onToggleGroup(leftovers.filter(toggleable).map((l) => l.path), true)}
          className="text-[11px] font-semibold shrink-0"
          style={{ color: ACCENT_GREEN }}
        >
          {t('uninstall.leftover.selectAll')}
        </button>
      </div>

      {preview?.app?.is_official_uninstaller && (
        <div className="mx-3 mt-2 px-2.5 py-1.5 rounded-lg bg-yellow-500/10 border border-yellow-500/20 flex items-start gap-1.5">
          <AlertTriangle size={12} className="text-yellow-400 mt-0.5 shrink-0" />
          <span className="text-[11px] text-yellow-400">
            {t('uninstall.leftover.officialUninstaller', {
              vendor: preview.app.official_vendor || t('uninstall.leftover.officialVendor'),
            })}
          </span>
        </div>
      )}

      {preview?.manual_removal && (
        <div className="mx-3 mt-2 px-2.5 py-1.5 rounded-lg bg-yellow-500/10 border border-yellow-500/20 flex items-start gap-1.5">
          <AlertTriangle size={12} className="text-yellow-400 mt-0.5 shrink-0" />
          <span className="text-[11px] text-yellow-400">
            {t('uninstall.leftover.manualRemoval')}
          </span>
        </div>
      )}

      {previewLoading && (
        <div className="flex items-center gap-2 px-3 py-2 text-[10px] font-mono text-white/60">
          {t('uninstall.leftover.enumerating')}
        </div>
      )}

      {/* Auto selected 组 */}
      {autoGroup.length > 0 && (
        <div className="mt-2">
          {groupHeader(t('uninstall.leftover.group.autoSelected'), autoGroup)}
          {autoGroup.map(entryRow)}
        </div>
      )}

      {/* Needs review 组 */}
      {reviewGroup.length > 0 && (
        <div className="mt-1 border-t border-white/[0.12]">
          {groupHeader(t('uninstall.leftover.group.needsReview'), reviewGroup)}
          <p className="px-3 pb-0.5 text-[10px] font-mono text-white/60">
            {t('uninstall.leftover.reviewHint')}
          </p>
          {reviewGroup.map(entryRow)}
        </div>
      )}

      {/* Clear Data 模式：保留 app 本体，只清残留 */}
      <div className="mt-2 border-t border-white/[0.12] px-3 py-2">
        {cleaningState?.buttonState === 'loading' ? (
          // 正在清理：显示内嵌进度条
          <CleaningProgressBar
            percent={Math.round((cleaningState.progress!.currentIndex / cleaningState.progress!.totalCount) * 100)}
            currentAction={cleaningState.progress!.currentAction}
          />
        ) : cleaningState?.buttonState === 'success' ? (
          // 清理成功：显示成功状态
          <div className="flex items-center justify-between py-1">
            <div className="flex items-center gap-1.5">
              <CheckCircle2 size={14} className="text-emerald-400" />
              <span className="text-[11px] font-semibold text-emerald-400">
                {isUninstallMode ? t('uninstall.result.uninstallSuccess') : t('uninstall.result.cleanSuccess')}
              </span>
            </div>
            <span className="text-[10px] font-mono text-emerald-400">
              {t('uninstall.result.freed', { size: formatSize(cleaningState.result!.freedBytes) })}
            </span>
          </div>
        ) : cleaningState?.buttonState === 'error' ? (
          // 清理失败：显示失败状态 + 重试按钮
          <div className="flex items-center justify-between py-1">
            <div className="flex items-center gap-1.5">
              <XCircle size={14} className="text-red-400" />
              <span className="text-[11px] font-semibold text-red-400">
                {isUninstallMode ? t('uninstall.result.uninstallFailed') : t('uninstall.result.cleanFailed')}
              </span>
            </div>
            <button
              onClick={() => onClearDataOnly(entry)}
              className="text-[10px] text-red-400 hover:text-red-300 underline"
            >
              {t('uninstall.result.retry')}
            </button>
          </div>
        ) : (
          // 默认：显示按钮
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-mono text-white/60">
              {t('uninstall.leftover.keepAppHint')}
            </span>
            <button
              onClick={() => onClearDataOnly(entry)}
              className={`flex items-center gap-1 px-2.5 py-1 rounded-md text-[11px] font-semibold cursor-pointer transition-all duration-150 border ${
                isDataOnlyMode
                  ? 'border-emerald-500 bg-emerald-500/25 shadow-[0_0_8px_rgba(16,185,129,0.3)]'
                  : 'border-emerald-500/30 bg-emerald-500/10 hover:bg-emerald-500/20 hover:border-emerald-500/50'
              } active:bg-emerald-500/30 active:scale-[0.97]`}
              style={{ color: ACCENT_GREEN }}
            >
              <Eraser size={12} />
              {isDataOnlyMode ? t('uninstall.leftover.dataModeActive') : t('uninstall.leftover.clearDataOnly')}
            </button>
          </div>
        )}
      </div>
    </div>
  )
}

// ── 导出：选择状态 ──

export interface UninstallSelection {
  checkedPaths: Set<string>
}

// ── 排序字段 ──
export type SortField = 'name' | 'size' | 'recent'

// ── 卸载 tab 主组件 ──

interface UninstallTabProps {
  apps: MoleListAppsEntry[]
  loading: boolean
  searchText: string
  selection: UninstallSelection
  sortField: SortField
  sortAscending: boolean
  onSelectionChange: (s: UninstallSelection) => void
  onClearDataOnly: (app: MoleListAppsEntry) => void
  cleaningApps: Map<string, CleaningAppState>
}

export function UninstallTab({ apps, loading, searchText, selection, sortField, sortAscending, onSelectionChange, onClearDataOnly, cleaningApps }: UninstallTabProps) {
  const tauri = useTauri()
  const { t, locale } = useI18n()
  const [expandedPath, setExpandedPath] = useState<string>('')
  // failed=true 的条目允许折叠后重试（避免一次失败就永远停在「1/1 app bundle」 ）
  const [detailCache, setDetailCache] = useState<Record<string, { preview: MoleUninstallResult | null; leftovers: LeftoverEntry[]; failed?: boolean }>>({})
  const [previewLoading, setPreviewLoading] = useState<Set<string>>(new Set())

  const { checkedPaths } = selection

  // 过滤 + 排序（字段与方向由壳层工具栏控制）
  const filteredApps = useMemo(() => {
    const q = searchText.trim().toLowerCase()
    const list = apps.filter(
      (a) =>
        !q ||
        a.display_name.toLowerCase().includes(q) ||
        a.name.toLowerCase().includes(q) ||
        a.path.toLowerCase().includes(q)
    )
    return [...list].sort((a, b) => {
      let cmp = 0
      if (sortField === 'size') {
        cmp = (a.size_bytes || 0) - (b.size_bytes || 0)
      } else if (sortField === 'recent') {
        cmp = (a.last_used_epoch || 0) - (b.last_used_epoch || 0)
      } else {
        cmp = a.display_name.localeCompare(b.display_name, locale)
      }
      return sortAscending ? cmp : -cmp
    })
  }, [apps, searchText, sortField, sortAscending, locale])

  const toggleCheck = useCallback(
    (path: string) => {
      const next = new Set(checkedPaths)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      onSelectionChange({ checkedPaths: next })
    },
    [checkedPaths, onSelectionChange]
  )

  const toggleExpand = useCallback((path: string) => {
    setExpandedPath((prev) => {
      const next = prev === path ? '' : path
      const cached = next ? detailCache[next] : undefined
      if (next && (!cached || cached.failed)) {
        const entry = apps.find((a) => a.path === next)
        if (entry) {
          // 先展示 App Bundle，再异步填充残留（对齐 Burrow previewLoading）
          setDetailCache((c) => ({ ...c, [next]: { preview: null, leftovers: buildLeftovers(entry, null) } }))
          setPreviewLoading((s) => new Set(s).add(next))
          tauri
            .mole_uninstall({ app_path: next, dry_run: true })
            .then((preview: MoleUninstallResult | null) => {
              setDetailCache((c) => ({
                ...c,
                [next]: { preview: preview ?? null, leftovers: buildLeftovers(entry, preview ?? null) },
              }))
            })
            .catch((err) => {
              // 失败标记 failed：保留 App Bundle 展示，折叠再展开可重试
              console.error('[Apps] 残留预览加载失败', err)
              setDetailCache((c) => ({
                ...c,
                [next]: { preview: null, leftovers: buildLeftovers(entry, null), failed: true },
              }))
            })
            .finally(() => {
              setPreviewLoading((s) => {
                const ns = new Set(s)
                ns.delete(next)
                return ns
              })
            })
        }
      }
      return next
    })
  }, [detailCache, apps, tauri])

  const togglePath = useCallback(
    (path: string) => toggleCheck(path),
    [toggleCheck]
  )

  const toggleGroup = useCallback(
    (paths: string[], checked: boolean) => {
      const next = new Set(checkedPaths)
      for (const p of paths) {
        if (checked) next.add(p)
        else next.delete(p)
      }
      onSelectionChange({ checkedPaths: next })
    },
    [checkedPaths, onSelectionChange]
  )

  return (
    <div className="h-full flex flex-col">
      {loading ? (
        <div className="flex-1 flex flex-col items-center justify-center text-white/60">
          <div className="text-sm">{t('uninstall.list.scanning')}</div>
        </div>
      ) : (
      <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
        <div className="py-1">
        {filteredApps.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-white/60">
            <div className="text-sm">{searchText ? t('uninstall.list.noMatch') : t('uninstall.list.empty')}</div>
          </div>
        ) : (
          <div className="space-y-0.5">
            {filteredApps.map((app) => {
              const displayName = app.display_name || app.name
              const isExpanded = expandedPath === app.path
              const isChecked = checkedPaths.has(app.path)
              const prettyPath = app.path.replace(/^\/Users\/[^/]+/, '~')
              const cache = detailCache[app.path]

              return (
                <div key={app.path} className="mr-[24px]">
                  {/* 应用行 */}
                  <div
                    className="flex items-center gap-2.5 px-[24px] py-2 rounded-lg transition-colors hover:bg-black/[0.25]"
                    onClick={() => toggleExpand(app.path)}
                  >
                    <button className="shrink-0 text-white/60">
                      {isExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                    </button>

                    <AppIcon name={displayName} path={app.path} size={28} />

                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="text-[13px] font-medium text-[var(--text-primary)] truncate">
                          {displayName}
                        </span>
                        {app.running && (
                          <span className="w-1.5 h-1.5 rounded-full bg-green-400 shrink-0" title={t('uninstall.list.running')} />
                        )}
                      </div>
                      <div className="text-[10px] font-mono text-white/60 truncate">
                        {app.version ? `v${app.version}` : ''}
                        {app.version && prettyPath ? ' · ' : ''}
                        {prettyPath}
                      </div>
                    </div>

                    {/* 右侧摘要：大小 / 残留文件数 */}
                    <span className="text-[11px] font-mono text-white/85 shrink-0">
                      {cache ? (
                        t('uninstall.list.filesAndSize', {
                          count: cache.leftovers.length,
                          size: formatSize(app.size_bytes || 0),
                        })
                      ) : (
                        app.size_human || formatSize(app.size_bytes || 0)
                      )}
                    </span>

                    {/* 勾选：与 Clean 列表统一的 MoleCheckbox（蓝底白勾） */}
                    <MoleCheckbox
                      checked={isChecked}
                      onClick={(e) => {
                        e.stopPropagation()
                        toggleCheck(app.path)
                      }}
                    />
                  </div>

                  {/* 行内展开：残留审阅 */}
                  {isExpanded && (
                    <LeftoverPanel
                      entry={app}
                      preview={cache?.preview ?? null}
                      leftovers={cache?.leftovers ?? []}
                      checkedPaths={checkedPaths}
                      previewLoading={previewLoading.has(app.path)}
                      cleaningState={cleaningApps.get(app.path)}
                      onTogglePath={togglePath}
                      onToggleGroup={toggleGroup}
                      onClearDataOnly={onClearDataOnly}
                      isUninstallMode={cleaningApps.get(app.path)?.operationType === 'uninstall'}
                    />
                  )}
                </div>
              )
            })}
          </div>
          )}
        </div>
      </SimpleBar>
      )}
    </div>
  )
}
