import { dashTheme } from './theme'
import { statusColor, tempStatus, fanStatus, diskStatus } from './statusColors'

interface StatusBarProps {
  cpuTemp?: number
  fanSpeed?: number
  diskFreeGb?: number
  diskTotalGb?: number
}

/**
 * 顶部状态条：三列布局，显示 CPU 温度、风扇转速、磁盘可用空间。
 * 对齐 Lemon Cleaner 的状态条风格：上方大数值（带状态颜色），下方标签。
 */
export function StatusBar({ cpuTemp, fanSpeed, diskFreeGb, diskTotalGb }: StatusBarProps) {
  // 计算状态级别
  const tempLevel = cpuTemp !== undefined ? tempStatus(cpuTemp) : 'normal'
  const fanLevel = fanSpeed !== undefined ? fanStatus(fanSpeed) : 'normal'
  // 磁盘状态基于可用百分比
  const diskAvailablePercent = diskFreeGb !== undefined && diskTotalGb
    ? (diskFreeGb / diskTotalGb) * 100
    : 100
  const diskLevel = diskFreeGb !== undefined ? diskStatus(diskAvailablePercent) : 'normal'

  return (
    <div
      className="flex shrink-0 items-center justify-around rounded-xl px-4 py-3"
      style={{ background: dashTheme.card, border: `1px solid ${dashTheme.cardBorder}` }}
    >
      {/* CPU 温度 */}
      <div className="text-center">
        <div className="text-2xl font-bold tabular-nums" style={{ color: statusColor(tempLevel) }}>
          {cpuTemp ?? 0}°C
        </div>
        <div className="mt-0.5 text-[10px]" style={{ color: dashTheme.textTertiary }}>
          CPU 温度
        </div>
      </div>

      {/* 风扇转速 */}
      <div className="text-center">
        <div className="text-2xl font-bold tabular-nums" style={{ color: statusColor(fanLevel) }}>
          {fanSpeed ?? 0}
        </div>
        <div className="mt-0.5 text-[10px]" style={{ color: dashTheme.textTertiary }}>
          风扇转速
        </div>
      </div>

      {/* 磁盘可用 */}
      <div className="text-center">
        <div className="text-2xl font-bold tabular-nums" style={{ color: statusColor(diskLevel) }}>
          {diskFreeGb !== undefined ? `${Math.round(diskFreeGb)}GB` : '—'}
        </div>
        <div className="mt-0.5 text-[10px]" style={{ color: dashTheme.textTertiary }}>
          磁盘可用
        </div>
      </div>
    </div>
  )
}
