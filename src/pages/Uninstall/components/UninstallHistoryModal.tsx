import { useState, useEffect } from 'react'
import { Modal, Button, List, Tag, Space } from 'antd'
import { HistoryOutlined, DeleteOutlined, FolderOpenOutlined } from '@ant-design/icons'
import useTauri from '@/hooks/useTauri'
import { moleMessage } from '@/components/ui'
import { formatSize } from '@/utils/format'
import type { UninstallHistoryRecord } from '@/types/uninstall-history'

interface UninstallHistoryModalProps {
  visible: boolean
  onClose: () => void
}

export function UninstallHistoryModal({ visible, onClose }: UninstallHistoryModalProps) {
  const tauri = useTauri()
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
      moleMessage.error('加载历史记录失败')
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
    Modal.confirm({
      title: '确认清空历史记录？',
      content: '此操作不可撤销，历史记录将被永久删除。',
      okText: '清空',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: async () => {
        try {
          await tauri.mole_clear_uninstall_history()
          setRecords([])
          moleMessage.success('历史记录已清空')
        } catch (err) {
          console.error('清空历史记录失败', err)
          moleMessage.error('清空历史记录失败')
        }
      },
    })
  }

  // 打开废纸篓
  const handleOpenTrash = async () => {
    try {
      await tauri.mole_reveal_in_trash()
    } catch (err) {
      console.error('打开废纸篓失败', err)
      moleMessage.error('打开废纸篓失败')
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

    if (diffMins < 1) return '刚刚'
    if (diffMins < 60) return `${diffMins} 分钟前`
    if (diffHours < 24) return `${diffHours} 小时前`
    if (diffDays < 7) return `${diffDays} 天前`
    return date.toLocaleDateString('zh-CN')
  }

  return (
    <Modal
      title={
        <Space>
          <HistoryOutlined />
          <span>删除历史</span>
        </Space>
      }
      open={visible}
      onCancel={onClose}
      width={700}
      footer={[
        <Button key="trash" icon={<FolderOpenOutlined />} onClick={handleOpenTrash}>
          打开废纸篓
        </Button>,
        <Button
          key="clear"
          icon={<DeleteOutlined />}
          danger
          onClick={handleClearHistory}
          disabled={records.length === 0}
        >
          清空历史
        </Button>,
        <Button key="close" type="primary" onClick={onClose}>
          关闭
        </Button>,
      ]}
    >
      {records.length === 0 ? (
        <div style={{ textAlign: 'center', padding: '40px 0', color: '#999' }}>
          <HistoryOutlined style={{ fontSize: 48, marginBottom: 16 }} />
          <p>暂无删除历史</p>
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
                    {record.data_only && <Tag color="blue">只清数据</Tag>}
                    {!record.data_only && <Tag color="red">完整卸载</Tag>}
                  </Space>
                }
                description={
                  <div style={{ fontSize: 12, color: '#999' }}>
                    <div>{formatTime(record.timestamp)}</div>
                    <div>
                      {record.file_count} 个文件 · {formatSize(record.total_size_bytes)}
                    </div>
                    <div>
                      <span style={{ color: '#52c41a' }}>
                        {record.trashed_count} 个在废纸篓
                      </span>
                      {record.sudo_removed_count > 0 && (
                        <span style={{ color: '#ff4d4f', marginLeft: 8 }}>
                          {record.sudo_removed_count} 个永久删除
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
