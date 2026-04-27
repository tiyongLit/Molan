import { useState, useCallback, type ReactNode } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { HddOutlined } from '@ant-design/icons'
import { humanDiskSize } from '@/utils/format'
import { deriveDiskMetrics } from '@/utils/platform'
import type { DiskStatus } from '@/types/mole'

const MACOS_SYSTEM_STORAGE_URL = 'x-apple.systempreferences:com.apple.settings.Storage'

interface DashboardDiskCardProps {
  disk: DiskStatus | undefined
  volumeIconSrc: string | null
  openStorageSettingsSupported: boolean
  accentColor?: string
  children?: ReactNode
  className?: string
}

export function DashboardDiskCard({
  disk,
  volumeIconSrc,
  openStorageSettingsSupported,
  accentColor: _accentColor = '#60a5fa',
  children,
  className = ''
}: DashboardDiskCardProps) {
  const mount = '/'
  const volumeLabel = 'Macintosh HD'

  // 统一派生层：free/used/total/usedPercent 全部来自 deriveDiskMetrics()，
  // 与 Dashboard/Analyze 三处共享同一口径（diskFreeBytes → NSURLVolumeAvailableCapacityForImportantUsageKey）
  const { free, used, total, usedPercent } = deriveDiskMetrics(disk)
  const pct = disk ? Math.min(100, Math.max(0, usedPercent)) : 0
  // 未加载时统一 '—' 占位（对齐 StatusBar/LocationSelector），禁止伪造 0 值
  const freeLabel = disk ? humanDiskSize(free) : '—'
  const usedLabel = disk ? humanDiskSize(used) : '—'
  const totalLabel = disk ? humanDiskSize(total) : '—'

  const [hoverUsed, setHoverUsed] = useState(false)
  const [hoverAvailable, setHoverAvailable] = useState(false)
  const [hoverName, setHoverName] = useState(false)

  const handleOpenStorage = useCallback((e: React.MouseEvent) => {
    e.stopPropagation()
    openUrl(MACOS_SYSTEM_STORAGE_URL).catch((err) =>
      console.error('[DashboardDiskCard] open storage settings', err)
    )
  }, [])

  const cardBg = 'rgba(0, 0, 0, 0.2)'
  const cardBorder = 'rgba(118, 203, 244, 0.1)'
  const textPrimary = '#ffffff'
  const textSecondary = '#b9c4d3'
  const textHighlight = '#ffffff'
  const progressTrackBg = 'rgba(0, 0, 0, 0.25)'

  return (
    <div
      className={`w-full max-w-[500px] rounded-xl p-4 ${className}`}
      style={{
        background: cardBg,
        border: `1px solid ${cardBorder}`,
        boxShadow: '0 1px 6px rgba(0,0,0,0.18), inset 0 0.5px 0 rgba(255,255,255,0.08)'
      }}
    >
      <div className="flex items-center gap-4">
        <div className="flex h-[70px] w-[70px] shrink-0 items-center justify-center">
          {volumeIconSrc ? (
            <img
              src={volumeIconSrc}
              alt=""
              className="max-h-[64px] max-w-[64px] object-contain"
              draggable={false}
            />
          ) : (
            <HddOutlined style={{ fontSize: 42, color: textSecondary }} />
          )}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-2">
            {openStorageSettingsSupported ? (
              <button
                type="button"
                className="truncate text-left text-lg font-bold bg-transparent p-0 border-0 cursor-pointer"
                style={{
                  color: hoverName ? textHighlight : textPrimary,
                  textDecoration: 'none',
                  transition: 'color 0.2s'
                }}
                onClick={handleOpenStorage}
                onMouseEnter={() => setHoverName(true)}
                onMouseLeave={() => setHoverName(false)}
              >
                {volumeLabel}
              </button>
            ) : (
              <span className="truncate text-lg font-bold" style={{ color: textPrimary }}>
                {volumeLabel}
              </span>
            )}
            <span className="shrink-0 text-xs tabular-nums -mt-1" style={{ color: textSecondary }}>
              {disk ? `${pct.toFixed(0)}% full` : '—'}
            </span>
          </div>

          <div className="mt-1 space-y-0.5">
            <div className="flex items-center gap-2 text-xs">
              <span style={{ color: textSecondary }}>Location:</span>
              <span style={{ color: textPrimary }}>{mount}</span>
            </div>
            <div className="flex items-center gap-2 text-xs">
              <span style={{ color: textSecondary }}>Available:</span>
              <span
                className="transition-colors duration-200"
                style={{ color: hoverAvailable ? textHighlight : textSecondary }}
              >
                {freeLabel}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div className="flex items-center gap-3">
        <span
          className="shrink-0 text-[11px] tabular-nums transition-colors duration-200"
          style={{ color: hoverUsed ? textHighlight : textSecondary }}
        >
          {usedLabel}
        </span>

        <div className="relative flex-1">
          <div
            className="h-[10px] rounded-[5px] p-[3px]"
            style={{ background: progressTrackBg, boxShadow: 'inset 0 1px 2px rgba(0,0,0,0.3)' }}
          >
            <div className="relative h-full rounded-[2px] overflow-hidden flex">
              <div
                className="h-full rounded-[2px] transition-all duration-700 ease-out"
                style={{
                  width: `${pct}%`,
                  background: `linear-gradient(90deg, ${textHighlight}cc, ${textHighlight})`,
                  boxShadow: `0 0 8px ${textHighlight}40`
                }}
              />
            </div>
          </div>

          <div className="absolute inset-0 flex rounded-[5px] overflow-hidden">
            <div
              className="h-full cursor-default"
              style={{ width: `${pct}%` }}
              onMouseEnter={() => setHoverUsed(true)}
              onMouseLeave={() => setHoverUsed(false)}
            />
            <div
              className="flex-1 h-full cursor-default"
              onMouseEnter={() => setHoverAvailable(true)}
              onMouseLeave={() => setHoverAvailable(false)}
            />
          </div>
        </div>

        <span className="shrink-0 text-[11px] tabular-nums" style={{ color: textSecondary }}>
          {totalLabel}
        </span>
      </div>

      {children && <div className="mt-4">{children}</div>}
    </div>
  )
}
