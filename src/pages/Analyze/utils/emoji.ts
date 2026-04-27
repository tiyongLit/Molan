import type { MoleAnalyzeEntry } from '@/types/mole'

export const LOCATION_EMOJI: Record<string, string> = {
  'iOS Backups': '📱',
  'Old Downloads (90d+)': '📥',
  'System Logs': '📋',
  'Xcode DerivedData': '🔨',
  'Xcode Archives': '🔨',
  'Xcode Simulators': '📲',
  'Docker Data': '🐳',
  Home: '🏠',
  'App Library': '📁',
  Applications: '💻',
  'System Library': '⚙️'
}

export const DEFAULT_EMOJI = '📁'
export const INSIGHT_EMOJI = '💾'

export function getEmoji(entry: MoleAnalyzeEntry): string {
  return LOCATION_EMOJI[entry.name] ?? (entry.insight ? INSIGHT_EMOJI : DEFAULT_EMOJI)
}
