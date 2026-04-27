import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'

// 1 = 往下点（新页面从底部往上翻）, -1 = 往上点（新页面从顶部往下翻）
interface ShellNavContextType {
  direction: number
  navigateTo: (id: string) => void
}

const ShellNavContext = createContext<ShellNavContextType>({
  direction: 0,
  navigateTo: () => {}
})

const NAV_ORDER = ['home', 'clean', 'uninstall', 'optimize', 'analyze']

export function ShellNavProvider({
  children,
  currentId,
  onNavigate
}: {
  children: ReactNode
  currentId: string
  onNavigate: (id: string) => void
}) {
  const [direction, setDirection] = useState(0)

  const navigateTo = useCallback(
    (id: string) => {
      const fromIdx = NAV_ORDER.indexOf(currentId)
      const toIdx = NAV_ORDER.indexOf(id)
      if (fromIdx !== -1 && toIdx !== -1 && fromIdx !== toIdx) {
        setDirection(toIdx > fromIdx ? 1 : -1)
      }
      onNavigate(id)
    },
    [currentId, onNavigate]
  )

  return (
    <ShellNavContext.Provider value={{ direction, navigateTo }}>
      {children}
    </ShellNavContext.Provider>
  )
}

export function useShellNav() {
  return useContext(ShellNavContext)
}
