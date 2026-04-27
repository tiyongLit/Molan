import { useCallback } from 'react'
import { Dropdown } from 'antd'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { MoleButton } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { formatSize } from '@/utils/format'
import { useAnalyze } from '../contexts/AnalyzeContext'
import { useAnalyzeSelection } from '../contexts/AnalyzeSelectionContext'
import { EntryRow } from './EntryRow'
import { isProtectedEntrySync } from '../utils/protected'
import { CTX_MENU_CLASS, CTX_MENU_STYLE } from '../typings'

/**
 * 条目列表：全部 / Top20 切换 + 可勾选条目
 *
 * 采用事件委托模式减少回调 props 传递：
 * 容器层统一处理 click / dblclick，EntryRow 只接收纯数据 props。
 * 右键菜单使用 antd Dropdown 包裹每个 EntryRow，与 Treemap RectBlock 方式统一。
 */
export function EntryList() {
  const ctx = useAnalyze()
  const sel = useAnalyzeSelection()
  const tauri = useTauri()
  const { iconMap } = ctx
  const { activeData, focusedIdx, showTop20, toggleTop20, toggleCheck, setFocusedIdx } = sel

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
      if (entry) ctx.onActivate(entry)
    },
    [activeData.items, ctx.onActivate]
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
            ? `大文件 ${fileCount} 项`
            : `目录 ${dirCount} · 总计 ${formatSize(activeData.total)}`}
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
            全部
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
          每条用 Dropdown 包裹以支持右键菜单；事件委托挂在内层 wrapper（冒泡路径不变）。
          bundle 叶子钻取按需扫描期间渲染骨架屏（对齐柠檬无加载态的瞬时感缺失补偿） */}
      {ctx.bundleLoading ? (
        <div className="flex-1 min-h-0 px-2 py-2 flex flex-col gap-2 overflow-hidden">
          <span className="text-[10px] text-white/40">正在加载应用内部结构…</span>
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
            <Dropdown
              key={entry.path}
              menu={{
                items: ctx.buildContextMenu(entry).items,
                className: CTX_MENU_CLASS,
                style: CTX_MENU_STYLE
              }}
              trigger={['contextMenu']}
              disabled={protectedEntry}
              onOpenChange={(open) => {
                if (open) setFocusedIdx(idx)
              }}
            >
              <div>
                <EntryRow
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
              </div>
            </Dropdown>
          )
        })}
        </div>
      </SimpleBar>
      )}
    </>
  )
}
