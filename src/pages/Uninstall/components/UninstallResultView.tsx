import { CheckCircle, XCircle, ArrowLeft } from 'lucide-react'
import { formatSize } from '@/utils/format'

interface UninstalledApp {
  name: string
  path: string
  size: number
  success: boolean
}

interface UninstallResultViewProps {
  apps: UninstalledApp[]
  totalFreed: number
  onBack: () => void
}

/**
 * 卸载结果视图：显示本次卸载的详情
 * 
 * 功能：
 * - 显示卸载成功/失败图标
 * - 显示卸载了哪些应用（列表）
 * - 显示每个应用的大小
 * - 显示总共释放的空间
 * - 提供"返回应用列表"按钮
 */
export function UninstallResultView({ apps, totalFreed, onBack }: UninstallResultViewProps) {
  const successCount = apps.filter(app => app.success).length
  const failedCount = apps.filter(app => !app.success).length
  const isAllSuccess = failedCount === 0

  return (
    <div className="flex flex-col items-center justify-center h-full px-8 py-12">
      {/* 成功/失败图标 */}
      <div className="mb-6">
        {isAllSuccess ? (
          <CheckCircle size={80} className="text-green-500" />
        ) : (
          <XCircle size={80} className="text-red-500" />
        )}
      </div>

      {/* 标题 */}
      <h2 className="text-3xl font-bold mb-2 text-white">
        {isAllSuccess ? '卸载完成' : '卸载完成（部分失败）'}
      </h2>

      {/* 统计信息 */}
      <p className="text-lg text-white/70 mb-8">
        已卸载 <span className="font-bold text-white">{successCount}</span> 个应用
        {failedCount > 0 && (
          <>，失败 <span className="font-bold text-red-400">{failedCount}</span> 个</>
        )}
      </p>

      {/* 应用列表 */}
      <div className="w-full max-w-2xl mb-8">
        <div className="bg-black/20 rounded-lg border border-white/10 overflow-hidden">
          {/* 表头 */}
          <div className="flex items-center justify-between px-4 py-3 bg-black/30 border-b border-white/10">
            <span className="text-sm font-medium text-white/80">应用名称</span>
            <span className="text-sm font-medium text-white/80">释放空间</span>
          </div>

          {/* 应用列表 */}
          <div className="max-h-96 overflow-y-auto">
            {apps.map((app, index) => (
              <div
                key={app.path}
                className={`flex items-center justify-between px-4 py-3 ${
                  index < apps.length - 1 ? 'border-b border-white/5' : ''
                }`}
              >
                <div className="flex items-center gap-3 flex-1 min-w-0">
                  {/* 状态图标 */}
                  {app.success ? (
                    <CheckCircle size={16} className="text-green-500 shrink-0" />
                  ) : (
                    <XCircle size={16} className="text-red-500 shrink-0" />
                  )}
                  
                  {/* 应用名称 */}
                  <span className="text-sm text-white truncate">{app.name}</span>
                </div>

                {/* 释放空间 */}
                <span className="text-sm text-white/70 font-mono shrink-0 ml-4">
                  {formatSize(app.size)}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* 总共释放空间 */}
      <div className="mb-8 text-center">
        <p className="text-sm text-white/60 mb-1">共释放空间</p>
        <p className="text-3xl font-bold text-green-400">
          {formatSize(totalFreed)}
        </p>
      </div>

      {/* 返回按钮 */}
      <button
        onClick={onBack}
        className="flex items-center gap-2 px-6 py-3 bg-emerald-500 hover:bg-emerald-600 text-white font-medium rounded-lg transition-colors"
      >
        <ArrowLeft size={18} />
        返回应用列表
      </button>
    </div>
  )
}
