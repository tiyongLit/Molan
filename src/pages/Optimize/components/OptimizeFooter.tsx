import { MoleButton } from '@/components/ui'

export interface OptimizeFooterProps {
  /** 恢复至默认勾选态 */
  onResetToDefault: () => void
}

/**
 * 底部操作区：分隔线 + 恢复至默认勾选态。
 * 由调用方控制渲染时机（仅 preview 阶段）。
 */
export default function OptimizeFooter({ onResetToDefault }: OptimizeFooterProps) {
  return (
    <div className="pb-[12px] shrink-0">
      <div className="h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-10 w-full" />
      <div className="flex justify-end pt-3 pb-6">
        <MoleButton
          size="small"
          color="default"
          variant="link"
          style={{ fontSize: 12, color: '#dce1e8' }}
          onClick={onResetToDefault}
        >
          恢复至默认勾选态
        </MoleButton>
      </div>
    </div>
  )
}
