import { useCallback } from 'react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { MoleButton } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { formatSize } from '@/utils/format'
import { useI18n } from '@/i18n'
import { useAnalyzeData } from '../contexts/AnalyzeDataContext'
import { useAnalyzeAction } from '../contexts/AnalyzeActionContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { EntryRow } from './EntryRow'
import { isProtectedEntrySync } from '../utils/protected'

/**
 * 条目列表：全部 / Top20 切换 + 可勾选条目
 *
 * **事件委托架构**：
 * - click / dblclick 统一在容器层处理，EntryRow 只接收纯数据 props
 * - 右键菜单由全局单例 AnalyzeContextMenu 接管（通过 data-path 属性识别目标）
 * - EntryRow 不再包裹 antd Dropdown，组件树深度大幅降低
 *
 * **性能收益**：
 * - 原方案：N 个条目 = N 个 Dropdown 实例，每次重渲染全部重建
 * - 新方案：0 个 Dropdown，右键菜单全局仅 1 个实例
 */
export function EntryList() {
  const { iconMap, bundleLoading } = useAnalyzeData()
  const { onActivate } = useAnalyzeAction()
  const sel = useAnalyzeSelection()
  const tauri = useTauri()
  const { activeData, focusedIdx, showTop20, toggleTop20, toggleCheck, setFocusedIdx } = sel
  const { t } = useI18n()

  const dirCount = activeData.items.filter((e) => e.is_dir).length
  const fileCount = activeData.items.length - dirCount

  /** 点击委托：Checkbox → toggleCheck，其余 → setFocusedIdx */
  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-idx]')
      if (!row || row.dataset.protected === 'true') return

      const idx = Number(row.dataset.idx)
      if ((e.target as HTMLElement).tagName === 'INPUT') {
        toggleCheck(idx)
        return
      }
      setFocusedIdx(idx)
    },
    [toggleCheck, setFocusedIdx]
  )

  /** 双击委托：钻入目录 */
  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-idx]')
      if (!row || row.dataset.protected === 'true') return

      const idx = Number(row.dataset.idx)
      const entry = activeData.items[idx]
      if (entry) onActivate(entry)
    },
    [activeData.items, onActivate]
  )

  /** 在 Finder 中显示（对齐 Uninstall/Clean 的 clean_reveal_in_finder 用法） */
  const handleReveal = useCallback(
    (path: string) => {
      tauri.clean_reveal_in_finder({ path }).catch(() => {})
    },
    [tauri]
  )

  return (
    <>
      {/* 状态栏 + 模式切换 */}
      <div className="flex items-center justify-between px-2 py-1.5 border-b border-white/[0.10] shrink-0 select-none">
        <span className="text-[10px] text-white/40">
          {showTop20
            ? t('analyze.list.top20Count', { count: fileCount })
            : t('analyze.list.dirSummary', { count: dirCount, size: formatSize(activeData.total) })}
        </span>

        <div className="flex items-center gap-0 rounded overflow-hidden bg-white/[0.06] border border-white/[0.10]">
          <MoleButton
            type="text"
            size="small"
            onClick={() => showTop20 && toggleTop20()}
            style={{
              fontSize: 10,
              height: 18,
              paddingInline: 8,
              borderRadius: 0,
              color: !showTop20 ? '#64dfa7' : 'rgba(255,255,255,0.45)',
              fontWeight: !showTop20 ? 600 : 400,
              background: !showTop20 ? 'rgba(100,223,167,0.18)' : 'transparent'
            }}
            className="analyze-toggle-btn"
          >
            {t('analyze.list.modeAll')}
          </MoleButton>
          <div className="w-px h-4 bg-white/[0.10]" />
          <MoleButton
            type="text"
            size="small"
            onClick={() => !showTop20 && toggleTop20()}
            style={{
              fontSize: 10,
              height: 18,
              paddingInline: 8,
              borderRadius: 0,
              color: showTop20 ? '#64dfa7' : 'rgba(255,255,255,0.45)',
              fontWeight: showTop20 ? 600 : 400,
              background: showTop20 ? 'rgba(100,223,167,0.18)' : 'transparent'
            }}
            className="analyze-toggle-btn"
          >
            Top 20
          </MoleButton>
        </div>
      </div>

      {/* 条目列表 — mole-scroll 贴 aside 右缘（默认隐藏，滚动/悬停时显示）；
          事件委托挂在内层 wrapper；右键菜单由全局 AnalyzeContextMenu 接管。
          bundle 叶子钻取按需扫描期间渲染骨架屏（对齐柠檬无加载态的瞬时感缺失补偿） */}
      {bundleLoading ? (
        <div className="flex-1 min-h-0 px-2 py-2 flex flex-col gap-2 overflow-hidden">
          <span className="text-[10px] text-white/40">{t('analyze.list.bundleLoading')}</span>
          {Array.from({ length: 8 }).map((_, i) => (
            <div key={i} className="h-8 rounded-lg bg-white/[0.06] animate-pulse shrink-0" />
          ))}
        </div>
      ) : (
        <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
          <div className="py-1" onClick={handleClick} onDoubleClick={handleDoubleClick}>
            {activeData.items.map((entry, idx) => {
              const protectedEntry = isProtectedEntrySync(entry)
              return (
                <EntryRow
                  key={entry.path}
                  idx={idx}
                  entry={entry}
                  icon={
                    iconMap[entry.path] ??
                    (entry.is_symlink ? '🔗' : entry.is_dir ? '📁' : '📄')
                  }
                  isChecked={activeData.checkedSet.has(idx)}
                  isProtected={protectedEntry}
                  isFocused={focusedIdx === idx}
                  showPath={showTop20}
                  onReveal={handleReveal}
                />
              )
            })}
          </div>
        </SimpleBar>
      )}
    </>
  )
}
