import { useCallback } from 'react'
import { FolderOpenOutlined, CopyOutlined, DeleteOutlined, EyeOutlined } from '@ant-design/icons'
import { invoke } from '@tauri-apps/api/core'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { isProtectedEntrySync } from '../utils/protected'
import { moleMessage } from '@/components/ui'
import { t } from '@/i18n'
import type { ContextMenuItem, MenuEntry } from '../typings'

/**
 * 右键菜单项构建器。
 * useCallback 稳定引用：避免 AnalyzeContext 的 value 每次渲染都重建、连累全部消费者。
 */
export function useContextMenu(trashEntry: (entry: MenuEntry) => void, trashing: boolean) {
  return useCallback(
    (entry: MenuEntry): { items: ContextMenuItem[] } => {
      const items: ContextMenuItem[] = [
        {
          key: 'copy',
          icon: <CopyOutlined />,
          label: t('analyze.menu.copyPath'),
          onClick: async () => {
            try {
              await navigator.clipboard.writeText(entry.path)
              moleMessage.success(t('analyze.menu.copied'))
            } catch {
              moleMessage.error(t('analyze.menu.copyFailed'))
            }
          }
        },
        {
          key: 'reveal',
          icon: <FolderOpenOutlined />,
          label: t('analyze.revealInFinder'),
          onClick: () =>
            revealItemInDir(entry.path).catch(() =>
              moleMessage.error(t('analyze.menu.revealFailed'))
            )
        },
        {
          key: 'quicklook',
          icon: <EyeOutlined />,
          label: t('analyze.menu.quickLook', { name: entry.name }),
          onClick: () =>
            invoke('mole_quick_look', { path: entry.path }).catch((e) => {
              console.error(e)
              moleMessage.error(t('analyze.menu.quickLookFailed'))
            })
        }
      ]

      // 受保护条目不显示"移到废纸篓"
      if (!isProtectedEntrySync(entry)) {
        items.unshift({
          key: 'trash',
          icon: <DeleteOutlined />,
          label: t('analyze.trash.action'),
          danger: true,
          disabled: trashing,
          onClick: () => trashEntry(entry)
        })
      }

      return { items }
    },
    [trashEntry, trashing]
  )
}
