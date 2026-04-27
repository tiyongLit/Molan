import { WarningOutlined } from '@ant-design/icons'
import type { MoleOptimizeDiagnostics } from '@/types/mole'

export interface DiagnosticBannerProps {
  /** 诊断结果；无瓶颈或缺少 primary 时返回 null */
  diagnostics?: MoleOptimizeDiagnostics
}

/**
 * 性能诊断瓶颈提示条（preview 阶段，检测到持续高 CPU 时展示）。
 * 无瓶颈时返回 null，由 ScanPageLayout 的 banner 插槽渲染。
 */
export default function DiagnosticBanner({ diagnostics }: DiagnosticBannerProps) {
  if (!diagnostics?.has_bottleneck || !diagnostics.primary) return null
  const { primary } = diagnostics

  return (
    <div className="mx-[52px] mt-3">
      <div className="flex items-start gap-2 px-3 py-2 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
        <WarningOutlined style={{ fontSize: 13, color: 'rgba(250,204,21,0.85)' }} className="mt-0.5 shrink-0" />
        <div className="min-w-0">
          <div className="text-[13px] font-medium text-[var(--text-primary)]">
            检测到持续高 CPU：{primary.label}
            <span className="ml-1 font-normal text-white/60">
              (~{primary.avg_cpu}%)
            </span>
          </div>
          {primary.note && (
            <div className="text-[12px] text-white/40 mt-0.5">
              {primary.note}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
