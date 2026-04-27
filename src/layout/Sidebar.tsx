import { useCallback } from 'react'
import { Home, Sparkles, Trash2, Zap, FolderSearch } from 'lucide-react'
import FloatingDock, { type NavItem } from './Dock'
import { navAccents } from './themeColors'
import { useShellNav } from './ShellNavContext'
import { useActiveId } from './routing'

// ── Nav Items ──

const NAV_ITEMS: NavItem[] = [
  { id: 'home',      label: 'Home',      icon: <Home size={18} />,        accent: navAccents.home },
  { id: 'clean',     label: 'Clean',     icon: <Sparkles size={18} />,    accent: navAccents.clean },
  { id: 'uninstall', label: 'Uninstall', icon: <Trash2 size={18} />,      accent: navAccents.uninstall },
  { id: 'optimize',  label: 'Optimize',  icon: <Zap size={18} />,         accent: navAccents.optimize },
  { id: 'analyze',   label: 'Analyze',   icon: <FolderSearch size={18} />, accent: navAccents.analyze },
]

export default () => {
  const activeId = useActiveId()
  const { navigateTo } = useShellNav()

  const handleNavigate = useCallback(
    (id: string) => {
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
        showSettings={false}
      />
    </div>
  )
}
