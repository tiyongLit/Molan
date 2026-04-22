import { useLocation } from 'react-router-dom'

export function useActiveId(): string {
  const { pathname } = useLocation()
  return pathname.split('/').pop() || 'home'
}
