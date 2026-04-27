import { Modal } from 'antd'

/** 卸载结果（对齐后端 mole_uninstall_batch 返回） */
export interface UninstallBatchResult {
  success_count: number
  failed_count: number
  total_cleaned_size_human?: string
  /** "name|vendor" 格式，需要官方卸载器而被跳过的 app */
  blocked_apps?: string[]
  /** 扫描期被拒（特权路径祖先可变等），需用户在 Finder 手动移除的 app */
  manual_removal_apps?: string[]
  /** 卸载时仍在运行的 app（文件已删，进程仍存活） */
  running_at_uninstall_apps?: string[]
  system_extension_warning_apps?: string[]
  background_item_leftovers?: string[]
  local_network_warning_apps?: string[]
  status?: string
  status_title?: string
}

/**
 * 弹窗展示卸载结果（成功/失败统计 + 各类警告分组）。
 *
 * 纯展示逻辑，独立于页面壳层（index.tsx），避免壳层承担 Modal 内容组装。
 */
export function showUninstallResult(result: UninstallBatchResult) {
  const warnings: { label: string; items: string[] }[] = []

  const blocked = (result.blocked_apps ?? []).map((s) => {
    const [name, vendor] = s.split('|')
    return vendor ? `${name}（需 ${vendor} 官方卸载器）` : name
  })
  if (blocked.length) warnings.push({ label: '未能卸载（需官方卸载器）', items: blocked })

  if (result.manual_removal_apps?.length) {
    warnings.push({ label: '无法安全卸载（请在 Finder 中手动移到废纸篓）', items: result.manual_removal_apps })
  }

  if (result.running_at_uninstall_apps?.length) {
    warnings.push({ label: '卸载时仍在运行（文件已删、进程仍在）', items: result.running_at_uninstall_apps })
  }
  if (result.system_extension_warning_apps?.length) {
    warnings.push({ label: '系统扩展可能残留', items: result.system_extension_warning_apps })
  }
  if (result.background_item_leftovers?.length) {
    warnings.push({ label: '后台项残留', items: result.background_item_leftovers })
  }
  if (result.local_network_warning_apps?.length) {
    warnings.push({ label: '声明了本地网络权限', items: result.local_network_warning_apps })
  }

  const title = result.failed_count > 0
    ? '卸载完成（有失败）'
    : warnings.length > 0
      ? '卸载完成（有提醒）'
      : '卸载完成'

  Modal.info({
    title,
    width: 480,
    okText: '知道了',
    content: (
      <div className="flex flex-col gap-3">
        <div className="text-sm">
          {result.total_cleaned_size_human ? (
            <>
              已释放 <strong className="text-white">{result.total_cleaned_size_human}</strong>
              {result.success_count > 0 ? ` · 成功 ${result.success_count} 个` : ''}
              {result.failed_count > 0 ? ` · 失败 ${result.failed_count} 个` : ''}
            </>
          ) : result.failed_count > 0 ? (
            <>失败 {result.failed_count} 个</>
          ) : (
            <>成功 {result.success_count} 个</>
          )}
        </div>
        {warnings.map((w) => (
          <div key={w.label}>
            <div className="text-xs font-medium text-yellow-400">{w.label}</div>
            <ul className="mt-1 space-y-0.5">
              {w.items.map((item) => (
                <li key={item} className="text-xs text-white/70">{item}</li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    ),
  })
}
