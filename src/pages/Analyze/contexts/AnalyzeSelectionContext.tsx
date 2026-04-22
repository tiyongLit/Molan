import { createContext, useContext } from 'react'
import type { ActiveData } from '../typings'

export interface AnalyzeSelectionContextValue {
  checkedSet: Set<number>
  fileCheckedSet: Set<number>
  toggleCheck: (idx: number) => void
  selectAll: () => void
  deselectAll: () => void
  activeData: ActiveData
  checkedStats: { count: number; size: number }
  hasSelection: boolean
  showTop20: boolean
  toggleTop20: () => void
  focusedIdx: number | null
  setFocusedIdx: React.Dispatch<React.SetStateAction<number | null>>
}

export const AnalyzeSelectionContext = createContext<AnalyzeSelectionContextValue | null>(null)

export function useAnalyzeSelection() {
  const ctx = useContext(AnalyzeSelectionContext)
  if (!ctx) throw new Error('useAnalyzeSelection must be used within AnalyzeProvider')
  return ctx
}
