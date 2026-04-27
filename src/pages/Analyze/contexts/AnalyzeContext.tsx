import {
  createContext,
  useContext,
  useState,
  useMemo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  memo
} from 'react'
import { Modal } from 'antd'
import { openPath } from '@tauri-apps/plugin-opener'
import { moleMessage } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { useNavigation } from '@/hooks/useNavigation'
import { useAnalyzeData } from '../hooks/useAnalyzeData'
import { useIconMap } from '@/hooks/useIcon'
import { useContextMenu } from '../hooks/useContextMenu'
import { parseBreadcrumb, largeFileToEntry } from '../utils/path'
import { isProtectedEntrySync } from '../utils/protected'
import { AnalyzeSelectionContext } from './AnalyzeSelectionContext'
import type { MoleAnalyzeEntry, MoleAnalyzeResult } from '@/types/mole'
import type {
  ActiveData,
  BreadcrumbItem,
  TreemapItem,
  IconInput,
  ContextMenuItem,
  MenuEntry
} from '../typings'
import type { AnalyzeSelectionContextValue } from './AnalyzeSelectionContext'
import { formatSize } from '@/utils/format'

// ── Filter match helper (Go filterMatches: name or displayPath contains query) ──

function filterMatches(name: string, path: string, query: string): boolean {
  const needle = query.toLowerCase()
  return name.toLowerCase().includes(needle) || path.toLowerCase().includes(needle)
}

// ── 删除确认（静态阶段用 antd Modal；接后端后可换回 Tauri 原生 ask） ──

function confirmTrash(message: string): Promise<boolean> {
  return new Promise((resolve) => {
    Modal.confirm({
      title: '移到废纸篓',
      content: message,
      okText: '移到废纸篓',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: () => resolve(true),
      onCancel: () => resolve(false)
    })
  })
}

// ── Treemap 条目上限 — 对标 Lemon Cleaner 的 30 条上限（setSpaceVie 中 arrNum < 30）。
// 每个方格含 Popover/Dropdown 两层 antd 包装，大目录数百条目全量渲染会卡顿；
// 超出上限的小条目仍在左侧列表可见，仅不进入可视化。 ──
const TREEMAP_MAX_ITEMS = 50

// ── Context 类型 ──

interface AnalyzeContextValue {
  // Navigation
  currentPath: string
  canGoBack: boolean
  canGoForward: boolean
  goBack: () => void
  goForward: () => void
  drillIn: (path: string) => void
  breadcrumbJump: (idx: number) => void
  backToOverview: () => void
  switchRoot: (path: string) => void

  // Data
  entries: MoleAnalyzeEntry[]
  browseLoading: boolean
  /** bundle 叶子钻取按需扫描中（前端据此渲染骨架屏而非空白列表） */
  bundleLoading: boolean
  scanProgress: import('../hooks/useAnalyzeData').ScanProgress | null
  refreshPath: (path: string) => Promise<void>
  totalFiles: number

  // Icon & Treemap
  iconMap: Record<string, string>
  treemapItems: TreemapItem[]
  breadcrumbItems: BreadcrumbItem[]

  // Actions
  onActivate: (entry: MoleAnalyzeEntry) => void
  buildContextMenu: (entry: MenuEntry) => { items: ContextMenuItem[] }

  // Trash actions
  trashSelected: () => void
  trashEntry: (entry: MenuEntry) => void
  trashing: boolean

  // Filter
  filterQuery: string
  filtering: boolean
  setFilterQuery: (q: string) => void
  setFiltering: (v: boolean) => void

  // Misc
  overviewResult: MoleAnalyzeResult
}

const AnalyzeContext = createContext<AnalyzeContextValue | null>(null)

export function useAnalyze() {
  const ctx = useContext(AnalyzeContext)
  if (!ctx) throw new Error('useAnalyze must be used within AnalyzeProvider')
  return ctx
}

// ── Provider ──

interface AnalyzeProviderProps {
  initialPath: string
  overviewResult: MoleAnalyzeResult
  onBackToOverview: () => void
  onSwitchRoot: (path: string) => void
  children: React.ReactNode
}

const AnalyzeProviderImpl = ({
  initialPath,
  overviewResult,
  onBackToOverview,
  onSwitchRoot,
  children
}: AnalyzeProviderProps) => {
  const tauri = useTauri()
  const nav = useNavigation(initialPath)

  const { browseData, browseLoading, scanProgress, fetchPath, refreshPath } = useAnalyzeData()

  // ── 模式 & 选中 ──
  const [checkedSet, setCheckedSet] = useState<Set<number>>(new Set())
  const [fileCheckedSet, setFileCheckedSet] = useState<Set<number>>(new Set())
  const [showTop20, setShowTop20] = useState(false)
  const [focusedIdx, setFocusedIdx] = useState<number | null>(null)
  const [filterQuery, setFilterQuery] = useState('')
  const [filtering, setFiltering] = useState(false)

  // 路径变化时清空选中和焦点
  useEffect(() => {
    setCheckedSet(new Set())
    setFileCheckedSet(new Set())
    setFocusedIdx(null)
  }, [nav.currentPath])

  // 加载数据 — useLayoutEffect 让 loading 状态在同一帧提交
  useLayoutEffect(() => {
    if (nav.currentPath) fetchPath(nav.currentPath)
  }, [nav.currentPath, fetchPath])

  const entries = browseData.entries

  // ── 图标 ──
  // 目录 + symlink（后端原样传路径给 iconForFile，symlink 由系统自带 alias 角标）+ 大文件
  const iconInputs: IconInput[] = useMemo(
    () => [
      ...entries
        .filter((e) => e.is_dir || e.is_symlink)
        .map((e) => ({ path: e.path, name: e.name, isDir: e.is_dir })),
      ...browseData.largeFiles.map((f) => ({ path: f.path, name: f.name, isDir: false }))
    ],
    [entries, browseData.largeFiles]
  )
  const iconMap = useIconMap(iconInputs)

  // ── 派生数据（被两个 Context 共同使用） ──

  const activeData = useMemo((): ActiveData => {
    if (!showTop20) {
      // 对齐 Lemon 展示口径：全部条目（含 0 大小条目与空目录）都进列表，
      // 不在数据层按 size 过滤。
      let items = entries
      if (filterQuery) {
        items = items.filter((e) => filterMatches(e.name, e.path, filterQuery))
      }
      return { items, checkedSet, total: browseData.totalSize }
    }
    let items = browseData.largeFiles.map(largeFileToEntry)
    if (filterQuery) {
      items = items.filter((e) => filterMatches(e.name, e.path, filterQuery))
    }
    const total = items.reduce((a, b) => a + b.size, 0)
    return { items, checkedSet: fileCheckedSet, total }
  }, [
    showTop20,
    entries,
    browseData.largeFiles,
    browseData.totalSize,
    checkedSet,
    fileCheckedSet,
    filterQuery
  ])

  const checkedStats = useMemo(() => {
    let count = 0
    let size = 0
    for (const idx of activeData.checkedSet) {
      if (idx < activeData.items.length) {
        count++
        size += activeData.items[idx].size
      }
    }
    return { count, size }
  }, [activeData])

  const hasSelection = checkedStats.count > 0

  const treemapItems = useMemo(
    () =>
      entries
        .filter((e) => e.size > 0)
        .sort((a, b) => b.size - a.size)
        .slice(0, TREEMAP_MAX_ITEMS)
        .map((e) => ({
          name: e.name,
          path: e.path,
          size: e.size,
          isDir: e.is_dir,
          protected: e.protected,
          icon:
            e.is_dir || e.is_symlink
              ? (iconMap[e.path] ?? (e.is_symlink ? '🔗' : '📁'))
              : e.name.endsWith('.app')
                ? '📦'
                : e.name.endsWith('.xcodeproj') || e.name.endsWith('.xcworkspace')
                  ? '🔨'
                  : '📄',
          rect: { x: 0, y: 0, width: 0, height: 0 }
        })),
    [entries, iconMap]
  )

  const breadcrumbItems = useMemo(() => {
    const rootEntry = overviewResult.entries.find((e) => e.path === initialPath)
    return parseBreadcrumb(initialPath, nav.currentPath, rootEntry)
  }, [initialPath, nav.currentPath, overviewResult.entries])

  // ── 选择操作 ──

  const showTop20Ref = useRef(showTop20)
  showTop20Ref.current = showTop20

  // 用 ref 持有 active items，供 toggleCheck/selectAll 同步读取
  const activeItemsRef = useRef(activeData.items)
  activeItemsRef.current = activeData.items

  const toggleCheck = useCallback((idx: number) => {
    // 受保护条目禁止勾选
    const items = activeItemsRef.current
    if (idx < items.length && isProtectedEntrySync(items[idx])) return

    const setter = showTop20Ref.current ? setFileCheckedSet : setCheckedSet
    setter((prev) => {
      const next = new Set(prev)
      if (next.has(idx)) next.delete(idx)
      else next.add(idx)
      return next
    })
  }, [])

  const selectAll = useCallback(() => {
    const items = activeItemsRef.current
    const count = items.length
    const allIdxs = new Set<number>()
    for (let i = 0; i < count; i++) {
      if (!isProtectedEntrySync(items[i])) {
        allIdxs.add(i)
      }
    }
    if (showTop20Ref.current) setFileCheckedSet(allIdxs)
    else setCheckedSet(allIdxs)
  }, [])

  const deselectAll = useCallback(() => {
    if (showTop20Ref.current) setFileCheckedSet(new Set())
    else setCheckedSet(new Set())
  }, [])

  const toggleTop20 = useCallback(() => {
    setShowTop20((v) => !v)
    setFocusedIdx(null)
  }, [])

  // ── 删除状态 ──
  const [trashing, setTrashing] = useState(false)

  // ── bundle 叶子钻取骨架屏 ──
  const [bundleLoading, setBundleLoading] = useState(false)
  useEffect(() => {
    if (!browseLoading) setBundleLoading(false)
  }, [browseLoading])

  const doTrash = useCallback(
    async (paths: string[], count: number) => {
      setTrashing(true)
      try {
        // 受保护兜底在后端（is_protected_entry_path），前端只发路径
        await tauri.mole_analyze_trash({ args: { paths } })
        moleMessage.success(`已将 ${count} 项移到废纸篓`)
        deselectAll()
        refreshPath(nav.currentPath)
      } catch (err: unknown) {
        console.error('[Analyze] doTrash failed:', err)
        moleMessage.error(err instanceof Error ? err.message : '删除失败')
      } finally {
        setTrashing(false)
      }
    },
    [tauri, deselectAll, refreshPath, nav.currentPath]
  )

  const activeDataRef = useRef(activeData)
  activeDataRef.current = activeData

  const trashSelected = useCallback(async () => {
    const data = activeDataRef.current
    const paths: string[] = []
    for (const idx of data.checkedSet) {
      if (idx < data.items.length) {
        const entry = data.items[idx]
        if (!isProtectedEntrySync(entry)) {
          paths.push(entry.path)
        }
      }
    }
    if (paths.length === 0) return

    let count = 0
    let size = 0
    for (const idx of data.checkedSet) {
      if (idx < data.items.length) {
        count++
        size += data.items[idx].size
      }
    }

    const confirmed = await confirmTrash(`确认将 ${count} 项 (${formatSize(size)}) 移到废纸篓？`)
    if (!confirmed) return

    doTrash(paths, count)
  }, [doTrash])

  const trashEntry = useCallback(
    async (entry: MenuEntry) => {
      if (isProtectedEntrySync(entry)) return

      const confirmed = await confirmTrash(`确认将 "${entry.name}" 移到废纸篓？`)
      if (!confirmed) return

      doTrash([entry.path], 1)
    },
    [doTrash]
  )

  // ── 动作 ──

  const onActivate = useCallback(
    (entry: MoleAnalyzeEntry) => {
      if (entry.is_dir) {
        // bundle 叶子：后端需按需子树扫描，先展示骨架屏
        if (entry.is_bundle_leaf) setBundleLoading(true)
        nav.drillIn(entry.path)
      } else {
        // 静态阶段 mock 路径打开会失败，静默处理；接后端后恢复错误提示
        openPath(entry.path).catch(() => {})
      }
    },
    [nav.drillIn]
  )

  const buildContextMenu = useContextMenu(trashEntry, trashing)

  // ── Context Values ──

  const analyzeValue = useMemo(
    (): AnalyzeContextValue => ({
      ...nav,
      entries,
      browseLoading,
      bundleLoading,
      scanProgress,
      refreshPath,
      totalFiles: browseData.totalFiles,
      iconMap,
      treemapItems,
      breadcrumbItems,
      onActivate,
      buildContextMenu,
      trashSelected,
      trashEntry,
      trashing,
      filterQuery,
      filtering,
      setFilterQuery,
      setFiltering,
      overviewResult,
      backToOverview: onBackToOverview,
      switchRoot: onSwitchRoot
    }),
    [
      nav,
      entries,
      browseLoading,
      bundleLoading,
      scanProgress,
      refreshPath,
      browseData.totalFiles,
      iconMap,
      treemapItems,
      breadcrumbItems,
      onActivate,
      buildContextMenu,
      trashSelected,
      trashEntry,
      trashing,
      filterQuery,
      filtering,
      overviewResult,
      onBackToOverview,
      onSwitchRoot
    ]
  )

  const selectionValue = useMemo(
    (): AnalyzeSelectionContextValue => ({
      checkedSet,
      fileCheckedSet,
      toggleCheck,
      selectAll,
      deselectAll,
      activeData,
      checkedStats,
      hasSelection,
      showTop20,
      toggleTop20,
      focusedIdx,
      setFocusedIdx
    }),
    [
      checkedSet,
      fileCheckedSet,
      toggleCheck,
      selectAll,
      deselectAll,
      activeData,
      checkedStats,
      hasSelection,
      showTop20,
      toggleTop20,
      focusedIdx
    ]
  )

  return (
    <AnalyzeContext.Provider value={analyzeValue}>
      <AnalyzeSelectionContext.Provider value={selectionValue}>
        {children}
      </AnalyzeSelectionContext.Provider>
    </AnalyzeContext.Provider>
  )
}

/** 用 memo 阻止父组件重渲染时连累 AnalyzeProvider 子树 */
export const AnalyzeProvider = memo(AnalyzeProviderImpl)
