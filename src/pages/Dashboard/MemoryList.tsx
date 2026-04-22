import { useEffect, useRef, useState } from 'react'
import SimpleBar from 'simplebar-react'
import { Tooltip } from 'antd'
import { InfoCircleOutlined } from '@ant-design/icons'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { moleMessage, MoleButton } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { nativeIconRegistry } from '@/utils/nativeIconRegistry'
import { useI18n } from '@/i18n'
import { dashTheme as theme } from './theme'

import 'simplebar-react/dist/simplebar.min.css'

export interface MemProcess {
  /** 稳定行标识（React key）：每 2s 一帧的活列表必须用不变身份，
   * 否则行组件每帧重建 → 水位条动画重启 + 图标重新挂载（视觉抽动） */
  pid: number
  name: string
  memoryLabel: string
  /** 占物理内存百分比（1~100） */
  percent: number
  /** 原生应用图标 SVG data URI（后端按 pid/ppid/name 匹配 bundle 后随帧下发，
   * 进程域与文件域共享同一份内容寻址结果）；缺省时走 bundlePath + 注册表预取 */
  icon?: string
  /** 应用 bundle 路径（.app）：注册表取图键，与 Uninstall 列表同源 */
  bundlePath?: string
}

/**
 * 内存占用详细区（腾讯柠檬 LMMemoryCellView 同款交互）：
 * 标题行显示"内存占用"与可用内存。
 * 列表 flex-1 填满气泡剩余空间 + 滚动（SimpleBar，默认隐藏轨道）。
 *
 * 行交互（对齐柠檬）：
 *   常态：左侧 icon + 进程名，右侧 内存占用数值 + 百分比
 *   hover 行：右侧替换为「关闭」按钮（UninstallTab 同款深色主题风格：
 *     半透明 border + bg + accent 文字色，hover/active 渐变反馈）
 *   按钮自带 tooltip：hover 显示 "关闭将直接退出程序，请先保存重要数据"
 *   点击「关闭」：调用 mole_kill_process(pid) 发送 SIGTERM，下一帧数据刷新自动移除该行
 *
 * 柠檬托盘进程列表布局参考：
 *   LMMemoryCellView 无固定 min/max 高度（由父 NSOutlineView 管理），
 *   最多展示 MAX_COUNT_ITEM=20 行；本组件由 Dashboard flex 布局自动约束，
 *   SimpleBar 负责溢出滚动（mole-scroll + -mr-4 pr-4 贴边技巧）。
 *
 * 图标（对齐 Uninstall 页模式，消除闪烁）：
 *   1. 后端随帧下发的 icon 优先直渲（零 IPC）；
 *   2. 缺省时按 bundlePath 走 nativeIconRegistry 批量解析，完成前先显示骨架屏，
 *      完成后由 AppIcon 内部的订阅自动换成真实图标；
 *   3. 单一图标源 + 稳定 key（pid），不存在「旧图标 → 新图标」跳变。
 */
export function MemoryList({ processes, availableGb, enableKillProcess = true }: { processes: MemProcess[]; availableGb?: number; enableKillProcess?: boolean }) {
  const tauri = useTauri()
  const { t } = useI18n()
  const [hoveredPid, setHoveredPid] = useState<number | null>(null)
  const [killingPid, setKillingPid] = useState<number | null>(null)

  // ── 图标预取（对齐 Uninstall：批量一次 IPC + 就绪后自动刷新）──
  const inflightRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    const paths: string[] = []
    for (const p of processes) {
      const path = p.bundlePath
      if (!path || p.icon) continue
      if (nativeIconRegistry.isResolved(path)) continue
      if (inflightRef.current.has(path)) continue
      paths.push(path)
    }
    if (paths.length === 0) return

    for (const path of paths) inflightRef.current.add(path)
    nativeIconRegistry
      .resolveIdle(paths)
      .catch((e: unknown) => console.error('[MemoryList] resolveIdle failed:', e))
      .finally(() => {
        for (const path of paths) inflightRef.current.delete(path)
      })
  }, [processes])

  // 进程退出后自动清除 killing 状态（下一帧 processes 更新时 killingPid 已不在列表中）
  useEffect(() => {
    if (killingPid !== null && !processes.some(p => p.pid === killingPid)) {
      setKillingPid(null)
    }
  }, [processes, killingPid])

  const handleKill = async (pid: number) => {
    if (killingPid !== null) return // 防连点
    setKillingPid(pid)
    try {
      await tauri.mole_kill_process({ pid })
      // 成功后不主动清除 killingPid，等 processes 更新后自动清除（useEffect）
    } catch (e) {
      console.error('[MemoryList] mole_kill_process failed:', e)
      moleMessage.error(t('dashboard.memory.kill_failed'))
      setKillingPid(null)
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="mb-1 flex items-center gap-2">
        <span className="text-[11px] font-medium" style={{ color: theme.textTertiary }}>
          {t('dashboard.memory.title')}
        </span>
        {availableGb !== undefined && (
          <span className="text-[11px] tabular-nums" style={{ color: theme.textSecondary }}>
            {t('dashboard.memory.available', { size: availableGb.toFixed(1) })}
          </span>
        )}
      </div>

      {/* 统一滚动条契约（对齐 Clean/Uninstall 的 SimpleBar 模式）：
          .mole-scroll 空闲隐藏、滚动/悬停显示；-mr-4 抵消父级右内边距使轨道贴气泡右缘，
          内层 pr-4 补偿内容留白；flex-1 min-h-0 + maxHeight:100% 与 UninstallTab 同款。 */}
      <SimpleBar className="mole-scroll -mr-4 min-h-0 flex-1" style={{ maxHeight: '100%' }}>
        <div className="flex flex-col gap-1.5 pr-4">
          {processes.map((p) => {
            const iconSrc = p.icon ?? null
            const isHovered = hoveredPid === p.pid
            const isKilling = killingPid === p.pid
            return (
              /* 柠檬 LMMemoryCellView 水位语法：整行即轨道（track 底色），
                 内部按占比叠一层 accent 水位色块，视觉上是「内存水位」而非细线进度条 */
              <div
                key={p.pid}
                className="relative overflow-hidden rounded"
                onMouseEnter={() => setHoveredPid(p.pid)}
                onMouseLeave={() => setHoveredPid(null)}
              >
                <div
                  className="absolute inset-y-0 left-0 transition-all duration-500"
                  style={{ width: `${p.percent}%`, background: theme.waterFill }}
                />
                <div className="relative flex items-center gap-2.5 p-1">
                  {/* 原生应用图标 */}
                  <AppIcon
                    name={p.name}
                    iconSrc={iconSrc}
                    path={iconSrc ? undefined : p.bundlePath}
                    size={22}
                  />
                  <span className="min-w-0 flex-1 truncate text-[12px]" style={{ color: theme.textPrimary }}>
                    {p.name}
                  </span>

                  {/* 右侧：常态显示 memoryLabel + 百分比，hover 时显示关闭按钮（受设置项控制）。
                      外层固定高度容器（h-6）消除两种状态的高度差，避免行高抖动。 */}
                  <div className="flex shrink-0 items-center" style={{ height: 24 }}>
                    {isHovered && enableKillProcess ? (
                      <Tooltip title={t('dashboard.memory.kill_warning')} placement="top">
                        <MoleButton
                          color='primary'
                          variant="link"
                          size="small"
                          iconPlacement="end"
                          loading={isKilling}
                          icon={<InfoCircleOutlined />}
                          onClick={() => handleKill(p.pid)}
                          style={{ fontSize: 11 }}
                        >
                          {isKilling ? t('dashboard.memory.killing') : t('dashboard.memory.kill')}
                        </MoleButton>
                      </Tooltip>
                    ) : (
                      <div className="flex items-center gap-2">
                        <span className="text-[11px] tabular-nums" style={{ color: theme.textSecondary }}>
                          {p.memoryLabel}
                        </span>
                        <span className="text-[10px] tabular-nums" style={{ color: theme.textTertiary }}>
                          {p.percent}%
                        </span>
                      </div>
                    )}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      </SimpleBar>
    </div>
  )
}
