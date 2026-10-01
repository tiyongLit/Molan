import { SEMANTIC_COLORS } from '@/constants/theme'

/**
 * 状态条 idle 错误占位：固定 min-height，
 * 避免错误出现/消失时下方列表发生位移抖动。
 */
export function ScanErrorPlaceholder({ error }: { error?: string }) {
  return (
    <div className="mt-2 min-h-[16px] flex items-center">
      {error ? (
        <p className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.dangerRed }}>{error}</p>
      ) : null}
    </div>
  )
}
