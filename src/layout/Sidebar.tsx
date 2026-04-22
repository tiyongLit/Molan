import { useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Home, Sparkles, Trash2, Zap, FolderSearch } from 'lucide-react'
import FloatingDock, { type NavItem } from './Dock'
import { navAccents } from './themeColors'
import { useShellNav } from './ShellNavContext'
import { useActiveId } from './routing'
import { preloadRoute } from '@/utils/preloadRoutes'
import { CMD_MOLE_OPEN_SETTINGS_WINDOW } from '@/constants/tauri-commands'

// ── Nav Items ──

const NAV_ITEMS: NavItem[] = [
  { id: 'home',      labelKey: 'nav.home',      icon: <Home size={18} />,        accent: navAccents.home },
  { id: 'clean',     labelKey: 'nav.clean',     icon: <Sparkles size={18} />,    accent: navAccents.clean },
  { id: 'uninstall', labelKey: 'nav.uninstall', icon: <Trash2 size={18} />,      accent: navAccents.uninstall },
  { id: 'optimize',  labelKey: 'nav.optimize',  icon: <Zap size={18} />,         accent: navAccents.optimize },
  { id: 'analyze',   labelKey: 'nav.analyze',   icon: <FolderSearch size={18} />, accent: navAccents.analyze },
]

export default () => {
  const activeId = useActiveId()
  const { navigateTo } = useShellNav()

  const handleNavigate = useCallback(
    (id: string) => {
      // 设置项：打开独立设置窗口（不路由到主窗口）
      if (id === 'settings') {
        invoke(CMD_MOLE_OPEN_SETTINGS_WINDOW).catch(() => {})
        return
      }
      navigateTo(id)
    },
    [navigateTo]
  )

  return (
    <div>
      <FloatingDock
        items={NAV_ITEMS}
        activeId={activeId}
        onNavigate={handleNavigate}
        onItemHover={preloadRoute}
        showSettings={true}
      />
    </div>
  )
}
