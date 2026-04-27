import { Progress } from 'antd'
import { formatSize } from '@/utils/format'
import { SpinnerRing } from './SpinnerRing'
import type { ScanProgress } from '../../hooks/useAnalyzeData'

/**
 * 扫描进度浮层 — 支持 null progress（扫描尚未收到进度事件时使用）。
 *
 * 统一视觉（开始分析 / 重新扫描 / 钻取扫描共用）：
 *   1. 进度环：中心内嵌两行 —— 百分比（字节口径，主视觉）+ 已扫描字节统计
 *   2. 辅助行：当前扫描路径（截断）
 */
export function ScanOverlay({ progress }: { progress: ScanProgress | null }) {
  // 尚未收到进度事件：纯 Spin 占位
  if (!progress) {
    return (
      <div className="flex flex-col items-center gap-4">
        <SpinnerRing size={72} />
        <span className="text-xs text-white/40">正在扫描目录...</span>
      </div>
    )
  }

  const hasPercent = progress.percent >= 0
  const percent = Math.min(progress.percent, 99)

  return (
    <div className="flex flex-col items-center gap-3 w-full max-w-[260px]">
      {/* ① 进度环：中心 format 内嵌百分比 + 已扫描字节（两行） */}
      {hasPercent ? (
        <Progress
          type="circle"
          percent={percent}
          size={120}
          status="active"
          strokeWidth={6}
          strokeColor={{
            '0%': 'var(--orange-400)',
            '100%': 'var(--orange-500)'
          }}
          format={() => (
            <div className="flex flex-col items-center justify-center gap-0.5 leading-none">
              {/* 主行：百分比（字节口径，环内主视觉） */}
              <span className="text-[28px] font-semibold font-mono tabular-nums text-white/90">
                {percent}%
              </span>
              {/* 次行：已扫描字节（实时统计） */}
              <span className="text-[10px] font-mono tabular-nums text-white/50 whitespace-nowrap">
                {formatSize(progress.bytes_scanned)}
              </span>
            </div>
          )}
        />
      ) : (
        <div className="flex flex-col items-center gap-3">
          <SpinnerRing size={72} />
          <span className="text-xs text-white/40">正在扫描目录...</span>
        </div>
      )}

      {/* ② 辅助行：当前路径（仅进度环可见时展示） */}
      {hasPercent && (
        <p
          className="text-[10px] text-white/35 max-w-[220px] truncate"
          title={progress.current_path}
        >
          {progress.current_path || '...'}
        </p>
      )}
    </div>
  )
}
