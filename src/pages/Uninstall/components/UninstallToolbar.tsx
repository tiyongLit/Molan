import type { ReactNode } from 'react'
import { ConfigProvider, Dropdown, theme } from 'antd'
import type { MenuProps } from 'antd'
import { Search, RefreshCw, History, Trash2 } from 'lucide-react'
import { useI18n } from '@/i18n'

/** 下拉菜单统一主题（卸载 tab 排序 / 启动 tab 范围筛选共用） */
const DROPDOWN_THEME = {
  algorithm: theme.darkAlgorithm,
  token: {
    colorBgElevated: 'rgba(28, 28, 40, 0.96)',
    colorText: 'rgba(255,255,255,0.88)',
    colorPrimary: '#64dfa7',
    colorPrimaryHover: '#7ff0bd',
  },
}

export interface UninstallToolbarProps {
  /** 搜索框当前值 */
  searchText: string
  /** 搜索框变化回调 */
  onSearchChange: (value: string) => void
  /** 搜索框占位文案 */
  searchPlaceholder: string
  /** 下拉触发器内容（图标 + 当前值 + 可选方向箭头） */
  trigger: ReactNode
  /** 下拉菜单项 */
  menuItems: MenuProps['items']
  /** 下拉选中 key */
  selectedKeys: string[]
  /** 下拉菜单点击回调（key 为菜单项 key） */
  onMenuClick: (key: string) => void
  /** 刷新按钮回调 */
  onRefresh: () => void
  /** 刷新按钮 hover 提示，默认「重新扫描」 */
  refreshTitle?: string
  /** 删除历史按钮回调（可选） */
  onShowHistory?: () => void
  /** 是否显示历史列表 */
  showHistory?: boolean
}

/**
 * 顶部工具栏：搜索框 + 下拉 + 刷新，卸载 / 启动项两个 tab 共用。
 *
 * 仅承担布局与样式，搜索 / 下拉 / 刷新语义由调用方通过 props 注入，
 * 消除 index.tsx 中两段近 40 行的逐字重复。
 */
export function UninstallToolbar({
  searchText,
  onSearchChange,
  searchPlaceholder,
  trigger,
  menuItems,
  selectedKeys,
  onMenuClick,
  onRefresh,
  refreshTitle,
  onShowHistory,
  showHistory = false,
}: UninstallToolbarProps) {
  const { t } = useI18n()
  return (
    <div className="flex items-stretch h-7 rounded-[3px] bg-black/[0.3] border border-white/[0.14] overflow-hidden">
      <div className="flex items-center gap-1.5 px-2.5">
        <Search size={12} className="text-[var(--text-secondary)]" />
        <input
          value={searchText}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={searchPlaceholder}
          className="bg-transparent outline-none text-xs text-[var(--text-primary)] placeholder-white/50 w-[110px]"
        />
      </div>
      <div className="w-px self-stretch my-1.5 bg-white/[0.12]" />
      <ConfigProvider theme={DROPDOWN_THEME}>
        <Dropdown
          menu={{
            items: menuItems,
            onClick: ({ key }) => onMenuClick(key),
            selectable: true,
            selectedKeys,
          }}
          trigger={['click']}
        >
          <button className="group flex items-center gap-1.5 px-2.5 text-xs text-white transition-opacity">
            {trigger}
          </button>
        </Dropdown>
      </ConfigProvider>
      <div className="w-px self-stretch my-1.5 bg-white/[0.12]" />
      <button
        onClick={onRefresh}
        title={refreshTitle ?? t('uninstall.rescan')}
        className="group flex items-center justify-center w-7 text-white transition-opacity"
      >
        <RefreshCw size={14} className="opacity-70 group-hover:opacity-100 transition-opacity" />
      </button>
      {onShowHistory && (
        <>
          <div className="w-px self-stretch my-1.5 bg-white/[0.12]" />
          <button
            onClick={onShowHistory}
            title={showHistory ? t('uninstall.toolbar.backToList') : t('uninstall.toolbar.viewHistory')}
            className="group flex items-center justify-center w-7 text-white transition-opacity"
          >
            {showHistory ? (
              <Trash2 size={14} className="opacity-70 group-hover:opacity-100 transition-opacity" />
            ) : (
              <History size={14} className="opacity-70 group-hover:opacity-100 transition-opacity" />
            )}
          </button>
        </>
      )}
    </div>
  )
}
