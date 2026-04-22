import { useState, useEffect } from 'react'
import { Modal, Button, List, Tag, Space } from 'antd'
import { HistoryOutlined, DeleteOutlined, FolderOpenOutlined } from '@ant-design/icons'
import useTauri from '@/hooks/useTauri'
import { moleMessage } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { formatSize } from '@/utils/format'
import { useI18n } from '@/i18n'
import type { UninstallHistoryRecord } from '@/types/uninstall-history'

interface UninstallHistoryModalProps {
  visible: boolean
  onClose: () => void
}

export function UninstallHistoryModal({ visible, onClose }: UninstallHistoryModalProps) {
  const tauri = useTauri()
  const { t, locale } = useI18n()
  const [records, setRecords] = useState<UninstallHistoryRecord[]>([])
  const [loading, setLoading] = useState(false)

  // 加载历史记录
  const loadHistory = async () => {
    setLoading(true)
    try {
      const result = await tauri.mole_get_uninstall_history() as { records: UninstallHistoryRecord[] }
      setRecords(result.records || [])
    } catch (err) {
      console.error('加载历史记录失败', err)
      moleMessage.error(t('uninstall.history.loadFailed'))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (visible) {
      loadHistory()
    }
  }, [visible])

  // 清空历史
  const handleClearHistory = async () => {
    const confirmed = await moleNativeConfirm(t('uninstall.history.clearConfirmTitle'), {
      informativeText: t('uninstall.history.clearConfirmBody'),
      kind: 'warning',
      okLabel: t('uninstall.history.clearOk'),
      cancelLabel: t('common.cancel'),
    })
    if (!confirmed) return

    try {
      await tauri.mole_clear_uninstall_history()
      setRecords([])
      moleMessage.success(t('uninstall.history.cleared'))
    } catch (err) {
      console.error('清空历史记录失败', err)
      moleMessage.error(t('uninstall.history.clearFailed'))
    }
  }

  // 打开废纸篓
  const handleOpenTrash = async () => {
    try {
      await tauri.mole_reveal_in_trash()
    } catch (err) {
      console.error('打开废纸篓失败', err)
      moleMessage.error(t('uninstall.history.openTrashFailed'))
    }
  }

  // 格式化时间
  const formatTime = (timestamp: string) => {
    const date = new Date(timestamp)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffMins = Math.floor(diffMs / 60000)
    const diffHours = Math.floor(diffMs / 3600000)
    const diffDays = Math.floor(diffMs / 86400000)

    if (diffMins < 1) return t('uninstall.time.justNow')
    if (diffMins < 60) return t('uninstall.time.minutesAgo', { count: diffMins })
    if (diffHours < 24) return t('uninstall.time.hoursAgo', { count: diffHours })
    if (diffDays < 7) return t('uninstall.time.daysAgo', { count: diffDays })
    return date.toLocaleDateString(locale)
  }

  return (
    <Modal
      title={
        <Space>
          <HistoryOutlined />
          <span>{t('uninstall.history.title')}</span>
        </Space>
      }
      open={visible}
      onCancel={onClose}
      width={700}
      footer={[
        <Button key="trash" icon={<FolderOpenOutlined />} onClick={handleOpenTrash}>
          {t('uninstall.history.openTrash')}
        </Button>,
        <Button
          key="clear"
          icon={<DeleteOutlined />}
          danger
          onClick={handleClearHistory}
          disabled={records.length === 0}
        >
          {t('uninstall.history.clear')}
        </Button>,
        <Button key="close" type="primary" onClick={onClose}>
          {t('common.close')}
        </Button>,
      ]}
    >
      {records.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '40px 0', color: '#999' }}>
          <HistoryOutlined style={{ fontSize: 48, marginBottom: 16 }} />
          <p>{t('uninstall.history.empty')}</p>
        </div>
      ) : (
        <List
          loading={loading}
          dataSource={records}
          renderItem={(record) => (
            <List.Item>
              <List.Item.Meta
                title={
                  <Space>
                    <span style={{ fontWeight: 600 }}>{record.app_name}</span>
                    {record.data_only && <Tag color="blue">{t('uninstall.history.dataOnly')}</Tag>}
                    {!record.data_only && <Tag color="red">{t('uninstall.history.fullUninstall')}</Tag>}
                  </Space>
                }
                description={
                  <div style={{ fontSize: 12, color: '#999' }}>
                    <div>{formatTime(record.timestamp)}</div>
                    <div>
                      {t('uninstall.list.filesAndSize', {
                        count: record.file_count,
                        size: formatSize(record.total_size_bytes),
                      })}
                    </div>
                    <div>
                      <span style={{ color: '#52c41a' }}>
                        {t('uninstall.history.trashed', { count: record.trashed_count })}
                      </span>
                      {record.sudo_removed_count > 0 && (
                        <span style={{ color: '#ff4d4f', marginLeft: 8 }}>
                          {t('uninstall.history.removed', { count: record.sudo_removed_count })}
                        </span>
                      )}
                    </div>
                  </div>
                }
              />
            </List.Item>
          )}
        />
      )}
    </Modal>
  )
}
