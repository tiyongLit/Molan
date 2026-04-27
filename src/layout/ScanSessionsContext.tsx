import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'

// ── 扫描会话状态（持久化在 Layout 层，页面切换不丢失） ──

export interface ScanSession {
  id: string
  status: 'idle' | 'scanning' | 'completed' | 'error'
  progress: number    // 0-100
  message?: string
  totalSize?: number
}

interface ScanSessionsContextValue {
  sessions: Record<string, ScanSession>
  registerSession: (session: ScanSession) => void
  updateSession: (id: string, patch: Partial<ScanSession>) => void
  getSession: (id: string) => ScanSession | undefined
  removeSession: (id: string) => void
}

const ScanSessionsContext = createContext<ScanSessionsContextValue | null>(null)

export function ScanSessionsProvider({ children }: { children: ReactNode }) {
  const [sessions, setSessions] = useState<Record<string, ScanSession>>({})

  const registerSession = useCallback((session: ScanSession) => {
    setSessions((prev) => ({ ...prev, [session.id]: session }))
  }, [])

  const updateSession = useCallback((id: string, patch: Partial<ScanSession>) => {
    setSessions((prev) => {
      const cur = prev[id]
      if (!cur) return prev
      return { ...prev, [id]: { ...cur, ...patch } }
    })
  }, [])

  const getSession = useCallback(
    (id: string) => sessions[id],
    [sessions],
  )

  const removeSession = useCallback((id: string) => {
    setSessions((prev) => {
      if (!(id in prev)) return prev
      const next = { ...prev }
      delete next[id]
      return next
    })
  }, [])

  return (
    <ScanSessionsContext.Provider value={{ sessions, registerSession, updateSession, getSession, removeSession }}>
      {children}
    </ScanSessionsContext.Provider>
  )
}

export function useScanSessions() {
  const ctx = useContext(ScanSessionsContext)
  if (!ctx) throw new Error('useScanSessions must be used within ScanSessionsProvider')
  return ctx
}
