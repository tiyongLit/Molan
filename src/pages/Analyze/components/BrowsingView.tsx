import { memo, useCallback, useRef, useEffect, useMemo } from 'react'
import { openPath } from '@tauri-apps/plugin-opener'
import { useAnalyze, AnalyzeProvider } from '../contexts/AnalyzeContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { Toolbar } from './Toolbar'
import { EntryList } from './EntryList'
import { Treemap } from './Treemap'
import { BottomBar } from './BottomBar'
import { useKeyboard } from '../hooks/useKeyboard'
import type { MoleAnalyzeResult } from '@/types/mole'
import type { TreemapItem } from '../typings'

/**
 * Browsing 阶段内部视图（需在 AnalyzeProvider 内渲染）
 */
function BrowsingInner() {
  const ctx = useAnalyze()
  const sel = useAnalyzeSelection()
  const containerRef = useRef<HTMLDivElement>(null)
  const filterInputRef = useRef<HTMLInputElement>(null)
  useKeyboard(containerRef)

  // 进入 filter 模式时自动聚焦输入框
  useEffect(() => {
    if (ctx.filtering && filterInputRef.current) {
      filterInputRef.current.focus()
    }
  }, [ctx.filtering])

  const progress = ctx.scanProgress

  const handleDrillIn = useCallback(
    (item: TreemapItem) => {
      if (item.isDir) {
        ctx.drillIn(item.path)
      } else {
        openPath(item.path).catch(() => {})
      }
    },
    [ctx.drillIn]
  )

  // ── Treemap 勾选/焦点改用 path 对齐 ──
  // treemapItems 经上限截断（TREEMAP_MAX_ITEMS）后 rect 索引不再与 activeData.items 索引对齐，
  // 以往 checkedSet.has(rectIdx) 在 filter 模式下本身就会错位；统一按 path 匹配彻底消除错位。
  const checkedPaths = useMemo(() => {
    const paths = new Set<string>()
    const { items, checkedSet } = sel.activeData
    for (const idx of checkedSet) {
      if (idx < items.length) paths.add(items[idx].path)
    }
    return paths
  }, [sel.activeData])

  // 键盘 ↑↓ 移动 focusedIdx 时，Treemap 对应方格同步高亮（反向联动）
  const focusedPath = useMemo(() => {
    const { items } = sel.activeData
    return sel.focusedIdx !== null && sel.focusedIdx < items.length
      ? items[sel.focusedIdx].path
      : null
  }, [sel.activeData, sel.focusedIdx])

  // 单击方格 → 同步左侧列表焦点（正向联动），后续 ↑↓/Space/Enter 从该条目继续
  const handleRectFocus = useCallback(
    (path: string) => {
      const idx = sel.activeData.items.findIndex((e) => e.path === path)
      if (idx >= 0) sel.setFocusedIdx(idx)
    },
    [sel.activeData, sel.setFocusedIdx]
  )

  return (
    <div ref={containerRef} tabIndex={-1} className="flex flex-col h-full outline-none">
      <Toolbar />

      {/* 主内容区 */}
      <div className="flex flex-1 min-h-0">
        <aside
          className={`${sel.showTop20 ? 'flex-1 min-w-0' : 'w-[280px] shrink-0'} flex flex-col border-r border-white/[0.10] transition-all duration-300 ease-out`}
        >
          {/* Filter bar — aligned with Go TUI / Filter mode */}
          {(ctx.filtering || ctx.filterQuery) && (
            <div className="flex items-center gap-2 px-3 py-2 border-b border-white/[0.10]">
              <span className="text-xs text-white/45">🔍</span>
              <input
                ref={filterInputRef}
                type="text"
                className="flex-1 bg-transparent text-sm text-white/85 outline-none placeholder:text-white/35"
                placeholder="Filter by name or path..."
                value={ctx.filterQuery}
                onChange={(e) => ctx.setFilterQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    ctx.setFiltering(false)
                    ctx.setFilterQuery('')
                  }
                }}
              />
              {ctx.filterQuery && (
                <span className="text-[10px] text-white/45 tabular-nums shrink-0">
                  {sel.activeData.items.length} matches
                </span>
              )}
            </div>
          )}
          <EntryList />
        </aside>

        <main
          className={`${sel.showTop20 ? 'w-0 opacity-0 overflow-hidden' : 'flex-1'} relative min-w-0 transition-all duration-300 ease-out bg-black/[0.12]`}
        >
          {!sel.showTop20 && (
            <Treemap
              items={ctx.treemapItems}
              onDrillIn={handleDrillIn}
              scanning={ctx.browseLoading}
              scanProgress={progress}
              checkedPaths={checkedPaths}
              focusedPath={focusedPath}
              onFocusPath={handleRectFocus}
              buildContextMenu={(item) => ctx.buildContextMenu(item)}
            />
          )}
        </main>
      </div>

      <BottomBar />
    </div>
  )
}

/**
 * Browsing 阶段视图：包裹 Context Provider
 */
interface BrowsingViewProps {
  initialPath: string
  overviewResult: MoleAnalyzeResult
  onBackToOverview: () => void
  onSwitchRoot: (path: string) => void
}

const BrowsingViewImpl = ({
  initialPath,
  overviewResult,
  onBackToOverview,
  onSwitchRoot
}: BrowsingViewProps) => {
  return (
    <AnalyzeProvider
      initialPath={initialPath}
      overviewResult={overviewResult}
      onBackToOverview={onBackToOverview}
      onSwitchRoot={onSwitchRoot}
    >
      <BrowsingInner />
    </AnalyzeProvider>
  )
}

/** 用 memo 阻止父组件（Analyze）因状态事件重渲染时连累 browsing 子树 */
export const BrowsingView = memo(BrowsingViewImpl)
