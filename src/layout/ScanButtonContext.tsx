import { createContext, useContext, useState, useCallback, useRef, type ReactNode } from 'react'

export interface ScanButtonState {
  visible: boolean
  percent: number
  label: string
  strokeColor: string
}

const DEFAULT_STATE: ScanButtonState = {
  visible: false,
  percent: 0,
  label: '',
  strokeColor: '#4B8DF8',
}

interface ScanButtonContextValue {
  state: ScanButtonState
  setState: (patch: Partial<ScanButtonState & { onClick?: () => void }>) => void
  triggerOnClick: () => void
}

const ScanButtonContext = createContext<ScanButtonContextValue | null>(null)

export function ScanButtonProvider({ children }: { children: ReactNode }) {
  const [state, setState_] = useState<ScanButtonState>(DEFAULT_STATE)
  const onClickRef = useRef<(() => void) | undefined>(undefined)

  const setState = useCallback(
    (patch: Partial<ScanButtonState & { onClick?: () => void }>) => {
      if ('onClick' in patch) {
        onClickRef.current = patch.onClick
      }
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { onClick: _, ...statePatch } = patch
      setState_((prev) => ({ ...prev, ...statePatch }))
    },
    [],
  )

  const triggerOnClick = useCallback(() => {
    onClickRef.current?.()
  }, [])

  return (
    <ScanButtonContext.Provider value={{ state, setState, triggerOnClick }}>
      {children}
    </ScanButtonContext.Provider>
  )
}

export function useScanButton() {
  const ctx = useContext(ScanButtonContext)
  if (!ctx) throw new Error('useScanButton must be used within ScanButtonProvider')
  return ctx
}
