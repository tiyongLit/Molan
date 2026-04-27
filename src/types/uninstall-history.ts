/** 卸载历史记录 */
export interface UninstallHistoryRecord {
  id: string
  timestamp: string
  app_name: string
  app_path: string
  data_only: boolean
  deleted_paths: string[]
  trashed_count: number
  sudo_removed_count: number
  total_size_bytes: number
  file_count: number
}

/** 卸载历史响应 */
export interface UninstallHistoryResponse {
  records: UninstallHistoryRecord[]
}
