import type { MoleAnalyzeEntry, MoleAnalyzeFile, MoleAnalyzeResult } from '@/types/mole'
import type { ItemType } from 'antd/es/menu/interface'

export interface BrowseData {
  entries: MoleAnalyzeEntry[]
  largeFiles: MoleAnalyzeFile[]
  totalSize: number
  totalFiles: number
}

export interface BreadcrumbItem {
  name: string
  path: string
  emoji?: string
}

export interface IconInput {
  path: string
  name: string
  isDir: boolean
}

export interface TreemapItem {
  name: string
  path: string
  size: number
  isDir: boolean
  icon: string
  rect: { x: number; y: number; width: number; height: number }
  protected?: boolean
}

/** 右键菜单与回收站操作所需的最小 entry 字段 */
export interface MenuEntry {
  path: string
  name: string
  protected?: boolean
}

export interface ActiveData {
  items: MoleAnalyzeEntry[]
  checkedSet: Set<number>
  total: number
}

/** 右键菜单项类型，复用 antd Menu 的 ItemType */
export type ContextMenuItem = ItemType

/** 右键菜单统一样式 — Dropdown menu.className */
export const CTX_MENU_CLASS = '!rounded-md !shadow-lg'

/** 右键菜单统一样式 — Dropdown popupClassName（外层包裹容器去边框） */
export const CTX_MENU_POPUP_CLASS = 'ctx-menu-popup'

/** 右键菜单统一样式 — Dropdown menu.style */
export const CTX_MENU_STYLE: React.CSSProperties = {
  background: 'rgba(28, 28, 40, 0.96)',
  backdropFilter: 'blur(12px)',
  border: 'none'
}

export type { MoleAnalyzeEntry, MoleAnalyzeFile, MoleAnalyzeResult }
