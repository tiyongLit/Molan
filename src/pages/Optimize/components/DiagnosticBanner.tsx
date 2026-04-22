import { WarningOutlined, AlertOutlined } from '@ant-design/icons'
import { useI18n } from '@/i18n'
import type { MoleOptimizeDiagnostics } from '@/types/mole'

export interface DiagnosticBannerProps {
  /** 诊断结果；无发现时返回 null */
  diagnostics?: MoleOptimizeDiagnostics
}

/** 格式化 KB 为人类可读 */
function formatKb(kb: number): string {
  if (kb >= 1_048_576) return `${(kb / 1_048_576).toFixed(1)} GB`
  if (kb >= 1024) return `${(kb / 1024).toFixed(0)} MB`
  return `${kb} KB`
}

const CARD_BASE = 'px-3 py-2 rounded-lg bg-yellow-500/10 border border-yellow-500/20'
const ICON_STYLE = { fontSize: 13, color: 'rgba(250,204,21,0.85)' } as const
const SUB_TEXT = 'text-[12px] text-white/40 mt-0.5'

/**
 * 性能诊断发现提示条（preview 阶段展示）。
 * 分层展示：CPU 瓶颈、内存压力、空闲虚拟机、失控进程。
 * 无任何发现时返回 null。
 */
export default function DiagnosticBanner({ diagnostics }: DiagnosticBannerProps) {
  const { t } = useI18n()

  if (!diagnostics?.has_bottleneck) return null

  const { primary, memory_pressure, idle_vm, runaway_processes } = diagnostics
  const hasAny = primary || memory_pressure || idle_vm || (runaway_processes && runaway_processes.length > 0)
  if (!hasAny) return null

  return (
    <div className="mx-[52px] mt-3 flex flex-col gap-2">
      {/* CPU 瓶颈 */}
      {primary && (
        <div className={`flex items-start gap-2 ${CARD_BASE}`}>
          <WarningOutlined style={ICON_STYLE} className="mt-0.5 shrink-0" />
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              {t('optimize.diag.highCpu', { label: primary.label })}
              <span className="ml-1 font-normal text-white/60">
                (~{primary.avg_cpu}%)
              </span>
            </div>
            {primary.note && (
              <div className={SUB_TEXT}>{primary.note}</div>
            )}
          </div>
        </div>
      )}

      {/* 内存压力 */}
      {memory_pressure && (
        <div className={`flex items-start gap-2 ${CARD_BASE}`}>
          <WarningOutlined style={ICON_STYLE} className="mt-0.5 shrink-0" />
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              {t('optimize.diag.memoryPressure', { pct: memory_pressure.swap_pct })}
              <span className="ml-1 font-normal text-white/60">
                {t('optimize.diag.memoryDetail', {
                  used: memory_pressure.swap_used_mb,
                  total: memory_pressure.swap_total_mb,
                  free: memory_pressure.free_pct,
                })}
              </span>
            </div>
            {memory_pressure.top_holders.length > 0 && (
              <div className={SUB_TEXT}>
                {t('optimize.diag.topHolders', { list: memory_pressure.top_holders.map(h => `${h.name} (${formatKb(h.rss_kb)})`).join(t('optimize.diag.holderSep')) })}
              </div>
            )}
            {memory_pressure.top_holders.length === 0 && (
              <div className={SUB_TEXT}>{t('optimize.diag.spread')}</div>
            )}
          </div>
        </div>
      )}

      {/* 空闲虚拟机 */}
      {idle_vm && (
        <div className={`flex items-start gap-2 ${CARD_BASE}`}>
          <WarningOutlined style={ICON_STYLE} className="mt-0.5 shrink-0" />
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              {t('optimize.diag.vmUsage', { size: formatKb(idle_vm.vm_kb) })}
              {idle_vm.docker_running === 0 && (
                <span className="ml-1 font-normal text-white/60">{t('optimize.diag.vmNoContainer')}</span>
              )}
              {idle_vm.docker_running !== null && idle_vm.docker_running > 0 && (
                <span className="ml-1 font-normal text-white/60">{t('optimize.diag.vmRunning', { count: idle_vm.docker_running })}</span>
              )}
            </div>
            {idle_vm.docker_running === 0 && (
              <div className={SUB_TEXT}>{t('optimize.diag.vmQuitHint')}</div>
            )}
            {idle_vm.docker_running === null && (
              <div className={SUB_TEXT}>{t('optimize.diag.vmCheckHint')}</div>
            )}
          </div>
        </div>
      )}

      {/* 失控进程 */}
      {runaway_processes && runaway_processes.length > 0 && (
        <div className={`flex items-start gap-2 ${CARD_BASE}`}>
          <AlertOutlined style={ICON_STYLE} className="mt-0.5 shrink-0" />
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              {t('optimize.diag.runawayTitle', { count: runaway_processes.length })}
            </div>
            {runaway_processes.map((proc) => (
              <div key={proc.pid} className={SUB_TEXT}>
                {t('optimize.diag.runawayLine', { name: proc.name, pid: proc.pid, runHours: proc.run_hours, cpuHours: proc.cpu_hours, pct: proc.pct })}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
