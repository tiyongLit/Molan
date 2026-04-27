import { useMemo, useRef, useState } from 'react'
import { Breadcrumb, Dropdown } from 'antd'
import type { MenuProps } from 'antd'
import { RightOutlined, MoreOutlined } from '@ant-design/icons'
import { useSize } from 'ahooks'
import { useAnalyze } from '../contexts/AnalyzeContext'
import { useIcon } from '@/hooks/useIcon'
import { computeVisibleItems } from '../utils/breadcrumb'
import { SegmentIcon } from './SegmentIcon'
import { MarqueeText } from '@/components/ui/MarqueeText'
import { CTX_MENU_CLASS, CTX_MENU_STYLE } from '../typings'
import type { MoleAnalyzeEntry } from '@/types/mole'
import type { CollapsedItem } from '../utils/breadcrumb'

const ICON_SIZE = 14

/** 原生系统图标优先（data URI → <img>），未命中降级 emoji —— 与 LocationSelector 同款 */
function renderIcon(src: string | null, emoji: string, size: number) {
  if (src && src.startsWith('data:')) {
    return (
      <img
        src={src}
        alt=""
        draggable={false}
        className="shrink-0 object-contain"
        style={{ width: size, height: size }}
      />
    )
  }
  return (
    <span className="shrink-0 leading-none select-none" style={{ fontSize: size }}>
      {emoji}
    </span>
  )
}

/** 折叠下拉菜单项图标 — 组件内调用 useIcon（React 合法用法） */
function MenuIcon({ path }: { path: string }) {
  const src = useIcon(path)
  return renderIcon(src, '📁', 14)
}

/**
 * 面包屑段按钮 — 图标（左）+ 文本（右）：
 * - 文本超宽（按钮 max-w-[160px]）时尾部 CSS 省略号
 * - 悬停按钮时跑马灯滚动展示完整内容（MarqueeText 受控模式）
 * - title 提示完整文件名
 */
function SegmentButton({
  label,
  icon,
  isLast,
  onClick
}: {
  label: string
  icon: React.ReactNode
  isLast: boolean
  onClick: () => void
}) {
  const [hovered, setHovered] = useState(false)

  return (
    <button
      onClick={onClick}
      disabled={isLast}
      title={label}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className={`
        flex items-center gap-1 px-1 rounded transition-colors duration-150 max-w-[160px] text-left
        ${
          isLast
            ? 'text-white/85 font-semibold cursor-default'
            : 'text-white/45 hover:text-white/70 hover:bg-black/[0.25]'
        }
      `}
    >
      {icon}
      <MarqueeText text={label} active={hovered} className="flex-1 min-w-0" />
    </button>
  )
}

/**
 * 面包屑导航 — 渲染层用 antd Breadcrumb，行为对齐 macOS NSPathControl（Finder）：
 * - 容器宽度自适应折叠（computeVisibleItems），超宽折叠头部中间段
 * - 省略号为下拉菜单入口：点开展示全部被折叠段（原生图标），点击直达任意层级
 * - 点击段跳转 breadcrumbJump；末项（当前目录）高亮不可点
 * - 首项渲染根目录原生图标（磁盘/主目录），其余段用 SegmentIcon（原生文件夹图标）
 */
export function PathBreadcrumb() {
  const ctx = useAnalyze()
  const items = ctx.breadcrumbItems

  const containerRef = useRef<HTMLDivElement>(null)
  const size = useSize(containerRef)
  const containerWidth = size?.width ?? 0

  const visibleItems = useMemo(
    () => computeVisibleItems(items, containerWidth),
    [items, containerWidth]
  )

  // 当前 root 的 overview entry（用于根目录图标与 label）
  const rootEntry = useMemo((): MoleAnalyzeEntry | undefined => {
    if (items.length === 0) return undefined
    return ctx.overviewResult.entries.find((e) => e.path === items[0].path)
  }, [items, ctx.overviewResult.entries])

  const rootPath = rootEntry?.path ?? items[0]?.path ?? ''
  const rootLabel = rootEntry?.name ?? (rootPath === '/' ? 'Macintosh HD' : rootPath)
  const rootIconSrc = useIcon(rootPath)
  const rootEmoji = rootPath === '/' ? '💾' : '📁'

  // 折叠下拉菜单：被折叠段逐项可跳转（key 为原始索引）
  const collapsedMenu = useMemo(() => {
    const build = (collapsed: CollapsedItem[]): MenuProps => ({
      items: collapsed.map((c) => ({
        key: String(c.originalIdx),
        icon: <MenuIcon path={c.path} />,
        label: <span className="text-xs text-white/70">{c.name}</span>
      })),
      onClick: (info) => ctx.breadcrumbJump(Number(info.key)),
      className: CTX_MENU_CLASS,
      style: CTX_MENU_STYLE
    })
    return build
  }, [ctx])

  // antd Breadcrumb items — title 放自渲染 button（交互完全自控，不受 antd 行为差异影响）
  const antdItems = useMemo(
    () =>
      visibleItems.map((item) => {
        if (item.ellipsis) {
          return {
            title: (
              <Dropdown menu={collapsedMenu(item.collapsed ?? [])} trigger={['click']}>
                <button
                  className="flex items-center justify-center w-5 h-5 rounded transition-colors duration-150 text-white/45 hover:text-white/80 hover:bg-black/[0.25]"
                  title="已折叠的路径层级"
                >
                  <MoreOutlined />
                </button>
              </Dropdown>
            )
          }
        }

        const originalIdx = item.originalIdx
        const isLast = originalIdx === items.length - 1
        const isRootSeg = originalIdx === 0

        return {
          title: (
            <SegmentButton
              label={isRootSeg ? rootLabel : item.name}
              isLast={isLast}
              onClick={() => ctx.breadcrumbJump(originalIdx)}
              icon={
                <span
                  className="flex items-center justify-center shrink-0"
                  style={{ width: ICON_SIZE + 2, height: ICON_SIZE + 2 }}
                >
                  {isRootSeg ? (
                    renderIcon(rootIconSrc, rootEmoji, ICON_SIZE)
                  ) : (
                    <SegmentIcon path={item.path} />
                  )}
                </span>
              }
            />
          )
        }
      }),
    [visibleItems, items.length, ctx, rootIconSrc, rootLabel, rootEmoji, collapsedMenu]
  )

  if (items.length === 0) return null

  return (
    <div ref={containerRef} className="h-full flex items-center overflow-hidden analyze-breadcrumb">
      <Breadcrumb
        separator={<RightOutlined className="text-white/40 select-none" style={{ fontSize: 8 }} />}
        items={antdItems}
      />
    </div>
  )
}
