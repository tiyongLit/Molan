import { FolderOpenOutlined, CopyOutlined, DeleteOutlined, EyeOutlined } from '@ant-design/icons'
import { invoke } from '@tauri-apps/api/core'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { isProtectedEntrySync } from '../utils/protected'
import { moleMessage } from '@/components/ui'
import type { ContextMenuItem, MenuEntry } from '../typings'

export function useContextMenu(trashEntry: (entry: MenuEntry) => void, trashing: boolean) {
  return (entry: MenuEntry): { items: ContextMenuItem[] } => {
    const items: ContextMenuItem[] = [
      {
        key: 'copy',
        icon: <CopyOutlined />,
        label: '复制路径',
        onClick: async () => {
          try {
            await navigator.clipboard.writeText(entry.path)
            moleMessage.success('路径已复制')
          } catch {
            moleMessage.error('复制失败')
          }
        }
      },
      {
        key: 'reveal',
        icon: <FolderOpenOutlined />,
        label: '在访达中显示',
        onClick: () =>
          revealItemInDir(entry.path).catch(() => moleMessage.error('无法在访达中显示'))
      },
      {
        key: 'quicklook',
        icon: <EyeOutlined />,
        label: `快速查看 "${entry.name}"`,
        onClick: () =>
          invoke('mole_quick_look', { path: entry.path }).catch((e) => {
            console.error(e)
            moleMessage.error('快速查看失败')
          })
      }
    ]

    // 受保护条目不显示"移到废纸篓"
    if (!isProtectedEntrySync(entry)) {
      items.unshift({
        key: 'trash',
        icon: <DeleteOutlined />,
        label: '移到废纸篓',
        danger: true,
        disabled: trashing,
        onClick: () => trashEntry(entry)
      })
    }

    return { items }
  }
}
