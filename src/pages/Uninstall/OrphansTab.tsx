import { useState, useCallback, useMemo, useLayoutEffect } from 'react'
import { createPortal } from 'react-dom'
import { Trash2, RefreshCw, Loader2, ShieldAlert, CheckCircle2 } from 'lucide-react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { Modal } from 'antd'
import useTauri from '@/hooks/useTauri'
import { moleMessage, MoleCheckbox } from '@/components/ui'
import { formatSize } from '@/utils/format'
import type { OrphanEntry, OrphanCategory, OrphanDeleteResult } from '@/types/mole'

// ── 分类标签与配色（对齐 UninstallTab 的 GROUP_LABEL_MAP 风格）──

const CATEGORY_LABEL: Record<OrphanCategory, string> = {
  cache: 'Cache',
  log: 'Log',
  saved_state: 'Saved State',
  http_storage: 'HTTP Storage',
  web_kit: 'WebKit',
  crash_reporter: 'Crash Report',
  preference: 'Preference',
  container: 'Container',
  launch_agent: 'Launch Agent',
  application_support: 'App Support',
  other: 'Other',
}

const CATEGORY_STYLE: Record<OrphanCategory, { color: string; bg: string }> = {
  cache: { color: '#60a5fa', bg: 'rgba(96,165,250,0.12)' },
  log: { color: '#a78bfa', bg: 'rgba(167,139,250,0.12)' },
  saved_state: { color: '#34d399', bg: 'rgba(52,211,153,0.12)' },
  http_storage: { color: '#fbbf24', bg: 'rgba(251,191,36,0.12)' },
  web_kit: { color: '#f472b6', bg: 'rgba(244,114,182,0.12)' },
  crash_reporter: { color: '#fb923c', bg: 'rgba(251,146,60,0.12)' },
  preference: { color: '#94a3b8', bg: 'rgba(148,163,184,0.12)' },
  container: { color: '#38bdf8', bg: 'rgba(56,189,248,0.12)' },
  launch_agent: { color: '#c084fc', bg: 'rgba(192,132,252,0.12)' },
  application_support: { color: '#4ade80', bg: 'rgba(74,222,128,0.12)' },
  other: { color: '#9ca3af', bg: 'rgba(156,163,175,0.12)' },
}

// ── 分类排序权重 ──
const CATEGORY_ORDER: OrphanCategory[] = [
  'cache',
  'log',
  'http_storage',
  'saved_state',
  'web_kit',
  'crash_reporter',
  'application_support',
  'preference',
  'container',
  'launch_agent',
  'other',
]

function CategoryBadge({ category }: { category: OrphanCategory }) {
  const s = CATEGORY_STYLE[category]
  return (
    <span
      className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium shrink-0"
      style={{ color: s.color, backgroundColor: s.bg }}
    >
      {CATEGORY_LABEL[category]}
    </span>
  )
}

export function OrphansTab() {
  const tauri = useTauri()
  const [orphans, setOrphans] = useState<OrphanEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [scanned, setScanned] = useState(false)
  const [checkedPaths, setCheckedPaths] = useState<Set<string>>(new Set())
  const [deleting, setDeleting] = useState(false)

  // 扫描按钮组通过 Portal 渲染到壳层顶行右上角（与卸载页工具栏同一行），
  // 状态与逻辑仍留在本组件，Portal 保留 React 树，事件/上下文不受影响。
  const [toolbarSlot, setToolbarSlot] = useState<HTMLElement | null>(null)
  useLayoutEffect(() => {
    setToolbarSlot(document.getElementById('orphans-toolbar-slot'))
  }, [])

  // ── 扫描 ──
  const handleScan = useCallback(async () => {
    setLoading(true)
    setScanned(false)
    setCheckedPaths(new Set())
    try {
      const result = await tauri.mole_orphan_scan() as OrphanEntry[]
      setOrphans(result ?? [])
      setScanned(true)
      // 自动勾选所有可删除项
      const autoChecked = new Set<string>()
      for (const o of result ?? []) {
        if (o.deletable) autoChecked.add(o.path)
      }
      setCheckedPaths(autoChecked)
    } catch (err) {
      console.error('[OrphansTab] scan failed', err)
      moleMessage.error('孤儿扫描失败: ' + String(err))
    } finally {
      setLoading(false)
    }
  }, [tauri])

  // ── 删除 ──
  const handleDelete = useCallback(async () => {
    const targets = Array.from(checkedPaths)
    if (targets.length === 0 || deleting) return

    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: `清理 ${targets.length} 个孤儿残留？`,
        content: (
          <div>
            <p className="mb-2">
              这些文件属于已卸载的应用，将移动到废纸篓（可恢复）。
            </p>
            <p className="text-xs text-gray-500">
              仅清理易失数据目录（Caches / Logs / HTTPStorages 等），不会触碰 Preferences 或 Containers。
            </p>
          </div>
        ),
        okText: '移入废纸篓',
        okButtonProps: { danger: true },
        cancelText: '取消',
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      })
    })
    if (!confirmed) return

    setDeleting(true)
    try {
      const result = await tauri.mole_orphan_delete({ paths: targets }) as OrphanDeleteResult
      const freed = formatSize(result.total_freed_bytes)
      if (result.failed_count > 0) {
        moleMessage.warning(
          `清理完成：${result.success_count} 成功，${result.failed_count} 失败，释放 ${freed}`
        )
      } else {
        moleMessage.success(`清理完成：${result.success_count} 项，释放 ${freed}`)
      }
      // 从列表中移除已删除项
      const deletedSet = new Set(targets.filter((_, i) => i < result.success_count))
      setOrphans((prev) => prev.filter((o) => !deletedSet.has(o.path)))
      setCheckedPaths(new Set())
    } catch (err) {
      console.error('[OrphansTab] delete failed', err)
      moleMessage.error('清理失败: ' + String(err))
    } finally {
      setDeleting(false)
    }
  }, [checkedPaths, deleting, tauri])

  // ── 勾选 ──
  const toggleCheck = useCallback((path: string, deletable: boolean) => {
    if (!deletable) return
    setCheckedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const toggleAll = useCallback(() => {
    const deletablePaths = orphans.filter((o) => o.deletable).map((o) => o.path)
    setCheckedPaths((prev) => {
      if (prev.size === deletablePaths.length) return new Set()
      return new Set(deletablePaths)
    })
  }, [orphans])

  // ── 按分类分组 ──
  const grouped = useMemo(() => {
    const map = new Map<OrphanCategory, OrphanEntry[]>()
    for (const cat of CATEGORY_ORDER) {
      map.set(cat, [])
    }
    for (const o of orphans) {
      const list = map.get(o.category)
      if (list) list.push(o)
      else map.get('other')!.push(o)
    }
    return Array.from(map.entries()).filter(([, items]) => items.length > 0)
  }, [orphans])

  const totalSize = useMemo(
    () => orphans.reduce((s, o) => s + o.size_bytes, 0),
    [orphans]
  )
  const deletableCount = useMemo(
    () => orphans.filter((o) => o.deletable).length,
    [orphans]
  )
  const checkedSize = useMemo(
    () =>
      orphans
        .filter((o) => checkedPaths.has(o.path))
        .reduce((s, o) => s + o.size_bytes, 0),
    [orphans, checkedPaths]
  )

  return (
    <div className="h-full flex flex-col">
      {/* 顶部统计栏：仅保留统计文案（扫描按钮组已 Portal 至壳层顶行右上角） */}
      <div className="shrink-0 flex items-center px-6 py-3">
        {scanned && orphans.length > 0 && (
          <span className="text-[11px] text-white/60">
            发现 <strong className="text-white">{orphans.length}</strong> 个孤儿残留 ·{' '}
            <strong className="text-white font-mono">{formatSize(totalSize)}</strong>
            {deletableCount < orphans.length && (
              <span className="text-white/40">
                {' '}（{deletableCount} 个可清理，{orphans.length - deletableCount} 个仅展示）
              </span>
            )}
          </span>
        )}
      </div>

      {/* 扫描完成后才出现：重新扫描 + 全选，Portal 到壳层顶行右上角（对齐卸载页工具栏） */}
      {toolbarSlot &&
        scanned &&
        createPortal(
          <div className="flex items-stretch h-7 rounded-[3px] bg-black/[0.3] border border-white/[0.14] overflow-hidden">
            <button
              onClick={handleScan}
              title="重新扫描"
              className="group flex items-center gap-1.5 px-2.5 text-xs text-white transition-colors hover:bg-white/[0.08]"
            >
              <RefreshCw size={13} className="opacity-70 group-hover:opacity-100 transition-opacity" />
              <span className="opacity-85 group-hover:opacity-100 transition-opacity">重新扫描</span>
            </button>
            {deletableCount > 0 && (
              <>
                <div className="w-px self-stretch my-1.5 bg-white/[0.12]" />
                <button
                  onClick={toggleAll}
                  className="group flex items-center px-2.5 text-xs text-white transition-colors hover:bg-white/[0.08]"
                >
                  <span className="opacity-85 group-hover:opacity-100 transition-opacity">
                    {checkedPaths.size === deletableCount ? '取消全选' : '全选可清理项'}
                  </span>
                </button>
              </>
            )}
          </div>,
          toolbarSlot
        )}

      {/* 内容区 */}
      <div className="flex-1 min-h-0">
        {!scanned && !loading && (
          <div className="h-full flex flex-col items-center justify-center gap-4 text-white/40">
            <ShieldAlert size={40} strokeWidth={1.2} />
            <div className="flex flex-col items-center gap-1.5">
              <p className="text-sm text-white/70">扫描已卸载应用留下的孤儿残留文件</p>
              <p className="text-xs text-white/25 max-w-xs text-center">
                反向扫描 Caches、Logs、HTTPStorages 等目录，
                找出不属于任何已安装应用的残留数据
              </p>
            </div>
            {/* 初始态唯一扫描入口；扫描完成后右上角 Portal 出「重新扫描」，二者共用 handleScan */}
            <button
              onClick={handleScan}
              className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold flex items-center gap-1.5"
            >
              <RefreshCw size={13} />
              扫描孤儿残留
            </button>
          </div>
        )}

        {loading && (
          <div className="h-full flex flex-col items-center justify-center gap-3 text-white/40">
            <Loader2 size={32} className="animate-spin" />
            <p className="text-sm">正在扫描孤儿残留…</p>
          </div>
        )}

        {scanned && !loading && orphans.length === 0 && (
          <div className="h-full flex flex-col items-center justify-center gap-3 text-white/40">
            <CheckCircle2 size={40} strokeWidth={1.2} className="text-emerald-400/60" />
            <p className="text-sm">没有发现孤儿残留</p>
            <p className="text-xs text-white/25">系统很干净</p>
          </div>
        )}

        {scanned && !loading && orphans.length > 0 && (
          <SimpleBar className="h-full" autoHide={false}>
            <div className="px-6 pb-4 pt-2">
              {grouped.map(([category, items]) => (
                <div key={category} className="mb-4">
                  {/* 分组标题：macOS 毛玻璃风格，-mx-6 扩展至 SimpleBar 全宽以消除边缘裁切 */}
                  <div
                    className="flex items-center gap-2 mb-2 sticky top-0 z-20 py-1.5  px-6 border-b border-white/[0.08] backdrop-blur rounded"
                    style={{ background: 'rgba(0,0,0,0.28)' }}
                  >
                    <CategoryBadge category={category} />
                    <span className="text-[10px] text-white/50">
                      {items.length} 项 · {formatSize(items.reduce((s, o) => s + o.size_bytes, 0))}
                    </span>
                  </div>

                  {/* 条目列表 */}
                  <div className="space-y-px">
                    {items.map((orphan) => {
                      const checked = checkedPaths.has(orphan.path)
                      return (
                        <div
                          key={orphan.path}
                          className={`flex items-center gap-2.5 px-3 py-2 rounded-md transition-colors cursor-default ${
                            orphan.deletable
                              ? 'hover:bg-white/[0.06]'
                              : 'opacity-50'
                          }`}
                          onClick={() => toggleCheck(orphan.path, orphan.deletable)}
                        >
                          {/* 勾选框 */}
                          {orphan.deletable ? (
                            <MoleCheckbox
                              checked={checked}
                              onClick={() => toggleCheck(orphan.path, true)}
                            />
                          ) : (
                            <span className="w-4 h-4 flex items-center justify-center shrink-0">
                              <ShieldAlert size={12} className="text-white/25" />
                            </span>
                          )}

                          {/* 文件名 + 路径 */}
                          <div className="flex-1 min-w-0">
                            <div className="text-xs text-white/90 truncate font-medium">
                              {orphan.file_name}
                            </div>
                            <div className="text-[10px] text-white/35 truncate font-mono">
                              {orphan.path}
                            </div>
                          </div>

                          {/* 大小 */}
                          <span className="text-[11px] text-white/50 font-mono shrink-0">
                            {orphan.size_human}
                          </span>

                          {/* 不可删除标记 */}
                          {!orphan.deletable && (
                            <span className="text-[9px] text-white/25 shrink-0 px-1.5 py-0.5 rounded bg-white/[0.04]">
                              仅展示
                            </span>
                          )}
                        </div>
                      )
                    })}
                  </div>
                </div>
              ))}
            </div>
          </SimpleBar>
        )}
      </div>

      {/* 底部操作栏 */}
      {scanned && checkedPaths.size > 0 && (
        <>
          <div className="shrink-0 h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-6" />
          <div className="shrink-0 flex items-center justify-between mx-6 h-[44px]">
            <span className="text-xs text-white/85">
              已选 <strong className="text-white">{checkedPaths.size}</strong> 项 ·{' '}
              <strong className="text-white font-mono">{formatSize(checkedSize)}</strong>
            </span>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setCheckedPaths(new Set())}
                className="apps-ghost-btn text-xs px-3 py-1.5 rounded-md transition-colors"
              >
                取消选择
              </button>
              <button
                onClick={handleDelete}
                disabled={deleting}
                className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold disabled:opacity-50 flex items-center gap-1.5"
              >
                {deleting ? (
                  <Loader2 size={13} className="animate-spin" />
                ) : (
                  <Trash2 size={13} />
                )}
                {deleting ? '清理中…' : `移入废纸篓 (${checkedPaths.size})`}
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
