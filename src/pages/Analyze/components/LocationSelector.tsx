import { useState, useEffect, useCallback, useMemo, type CSSProperties } from 'react'
import { Select } from 'antd'
import { DownOutlined } from '@ant-design/icons'
import { homeDir } from '@tauri-apps/api/path'
import { DiskProgressBar } from './DiskProgressBar'
import { useNativeIcon } from '@/hooks/useNativeIcon'
import { useI18n } from '@/i18n'
import { formatSize } from '@/utils/format'
import { deriveDiskMetrics } from '@/utils/platform'
import type { DiskStatus } from '@/types/mole'

interface LocationSelectorProps {
  value: string
  onChange: (value: string) => void
  disk: DiskStatus | undefined
}

const PICKER_VALUE = '__picker__'

/**
 * 选项图标：原生系统图标（data URI）优先渲染 <img>，未命中降级 emoji —— 永不白屏
 */
function renderOptionIcon(src: string | null, emoji: string, size: number) {
  if (src && src.startsWith('data:')) {
    return (
      <img
        src={src}
        alt=""
        draggable={false}
        className="shrink-0 object-contain"
        style={{ width: size, height: size }}
      />
    )
  }
  return (
    <span className="shrink-0 leading-none" style={{ fontSize: size }}>
      {emoji}
    </span>
  )
}

/**
 * LocationSelector — CleanMyMac X Space Lens 风格的目录选择器（浅色主题）
 *
 * 固定三个选项：Macintosh HD / 用户主目录 / 选择文件夹…
 * 选中 "选择文件夹…" 时调用系统文件夹选择对话框。
 */
export function LocationSelector({ value, onChange, disk }: LocationSelectorProps) {
  const [homePath, setHomePath] = useState<string>('')
  const [username, setUsername] = useState<string>('')
  const { locale, t } = useI18n()

  useEffect(() => {
    homeDir().then((dir) => {
      setHomePath(dir)
      const segments = dir.replace(/\/+$/, '').split('/')
      setUsername(segments[segments.length - 1] || dir)
    })
  }, [])

  const handlePicker = useCallback(async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const result = await open({ directory: true, multiple: false })
    if (result !== null) {
      onChange(result)
    }
  }, [onChange])

  const handleChange = useCallback(
    (val: string) => {
      if (val === PICKER_VALUE) {
        handlePicker()
      } else {
        onChange(val)
      }
    },
    [onChange, handlePicker],
  )

  const isCustomPath = useMemo(() => {
    if (!value) return false
    return value !== '/' && value !== homePath && value !== PICKER_VALUE
  }, [value, homePath])

  // ── 原生系统图标（注册表订阅 → mole_native_icons_resolve → NSWorkspace）──
  // 根目录 → Macintosh HD 内置磁盘图标；主目录 → Finder 原生渲染结果
  const rootIconSrc = useNativeIcon('/')
  const homeIconSrc = useNativeIcon(homePath)
  const customIconSrc = useNativeIcon(isCustomPath ? value : '')

  const options = useMemo(() => {
    const base = [
      { value: '/', label: 'Macintosh HD' },
      ...(homePath ? [{ value: homePath, label: username }] : []),
      { value: PICKER_VALUE, label: t('analyze.location.pick') },
    ]

    if (isCustomPath) {
      const segments = value.replace(/\/+$/, '').split('/')
      const basename = segments[segments.length - 1] || value
      base.splice(2, 0, { value, label: basename })
    }

    return base
  }, [homePath, username, value, isCustomPath, locale, t])

  // ── 选中态渲染（labelRender） ──
  const labelRender = useCallback(
    (option: any) => {
      const val = option.value as string

      // Macintosh HD — 绿色进度条
      if (val === '/') {
        // 统一派生层：与 Home/Dashboard 三处共享同一口径（deriveDiskMetrics → diskFreeBytes → NSURLVolumeAvailableCapacityForImportantUsageKey）
        const { free, used, total, usedPercent } = deriveDiskMetrics(disk)
        return (
          <div className="flex items-center gap-2.5 py-1 w-full">
            {renderOptionIcon(rootIconSrc, '💾', 64)}
            <div className="flex-1 min-w-0">
              <div className="flex items-baseline gap-1.5">
                <span className="text-[13px] font-semibold text-white/85">Macintosh HD</span>
                <span className="text-[11px] text-white/45 font-mono tabular-nums">
                  {total > 0 ? formatSize(total) : '—'}
                </span>
              </div>
              <div className="mt-1 mb-0.5">
                <DiskProgressBar
                  percent={usedPercent}
                  style={{
                    '--progress-track': 'rgba(0, 0, 0, 0.3)',
                    '--progress-fill-start': '#34c759',
                    '--progress-fill-end': '#30d158',
                  } as CSSProperties}
                />
              </div>
              <span className="text-[10px] text-white/45 font-mono tabular-nums">
                {t('analyze.location.diskUsage', {
                  free: free > 0 ? formatSize(free) : '—',
                  used: used > 0 ? formatSize(used) : '—'
                })}
              </span>
            </div>
          </div>
        )
      }

      // 用户主目录 — 无进度条，仅名称 + 描述（次级行与根目录“已使用”同款样式）
      if (val === homePath && homePath) {
        return (
          <div className="flex items-center gap-2.5 py-1 w-full">
            {renderOptionIcon(homeIconSrc, '📁', 64)}
            <div className="flex-1 min-w-0">
              <div className="flex items-baseline gap-1.5">
                <span className="text-[13px] font-semibold text-white/85">{username}</span>
              </div>
              <div className="mt-1 text-[10px] text-white/45">
                {t('analyze.location.homeFolder')}
              </div>
            </div>
          </div>
        )
      }

      // 自定义路径
      const segments = val.replace(/\/+$/, '').split('/')
      const basename = segments[segments.length - 1] || val
      return (
        <div className="flex items-center gap-2.5 py-1 w-full">
          {renderOptionIcon(customIconSrc, '📁', 64)}
          <div className="flex-1 min-w-0">
            <span className="text-[13px] font-semibold text-white/85 truncate block">{basename}</span>
            <div className="text-[10px] text-white/45 truncate">{val}</div>
          </div>
        </div>
      )
    },
    [disk, homePath, username, rootIconSrc, homeIconSrc, customIconSrc, t],
  )

  // ── 下拉项渲染（optionRender） ──
  const optionRender = useCallback(
    (option: any) => {
      const val = option.value as string
      const isSelected = val === value

      if (val === PICKER_VALUE) {
        return (
          <div className="flex items-center gap-2 py-0.5 px-0.5">
            <span className="text-[13px] text-white/70">{t('analyze.location.pick')}</span>
          </div>
        )
      }

      const isRoot = val === '/'
      const isHome = val === homePath && homePath !== ''
      const src = isRoot ? rootIconSrc : isHome ? homeIconSrc : customIconSrc
      const emoji = isRoot ? '💾' : '📁'
      const label = isRoot ? 'Macintosh HD' : username

      return (
        <div className="flex items-center gap-2 py-0.5 px-0.5">
          {renderOptionIcon(src, emoji, 18)}
          <span className="text-[13px] text-white/85 flex-1 truncate">{label}</span>
          {isSelected && (
            <span className="text-white/60 text-xs shrink-0">✓</span>
          )}
        </div>
      )
    },
    [value, homePath, username, rootIconSrc, homeIconSrc, customIconSrc, t],
  )

  return (
    <div className="flex w-full items-center border border-white/[0.14] bg-black/[0.18] backdrop-blur-md rounded-xl hover:bg-black/[0.22] transition-all duration-200">
      <Select
        value={value}
        onChange={handleChange}
        options={options}
        suffixIcon={<DownOutlined className="text-white/45 text-[11px]" />}
        optionRender={optionRender}
        labelRender={labelRender}
        classNames={{
          // antd 6 borderless 在 input:focus-visible 时会套 1px 主题色 outline，这里覆盖掉
          root: '!outline-none',
          popup: {
            // ── 弹出面板与触发器外壳 / 页面卡片完全同款：bg-black/[0.18] + backdrop-blur ──
            // 与外层 border-white/[0.14]、bg-black/[0.18] 一致，展开后视觉上“同一个盒子”
            root: '!bg-black/[0.18] !backdrop-blur-xl !border !border-white/[0.14] !shadow-dropdown !rounded-xl !p-1.5',
            listItem:
              '!rounded-lg !transition-all !duration-150 [&.ant-select-item-option-active]:!bg-white/[0.08] [&.ant-select-item-option-selected]:!bg-[rgba(59,130,246,0.15)]',
          },
        }}
        size="middle"
        variant="borderless"
        style={{ flex: 1, width: '100%' }}
      />
    </div>
  )
}
