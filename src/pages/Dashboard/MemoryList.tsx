import { useEffect, useRef, useState } from 'react'
import SimpleBar from 'simplebar-react'
import { Check, Loader2 } from 'lucide-react'
import { AppIcon } from '@/components/business/Apps/AppIcon'
import { moleMessage, MoleButton } from '@/components/ui'
import useTauri from '@/hooks/useTauri'
import { iconService } from '@/utils/iconService'
import { formatBytes } from './format'
import { dashTheme as t } from './theme'

import 'simplebar-react/dist/simplebar.min.css'

type ReleasePhase = 'idle' | 'running' | 'done'

export interface MemProcess {
  /** 稳定行标识（React key）：每 2s 一帧的活列表必须用不变身份，
   * 否则行组件每帧重建 → 水位条动画重启 + 图标重新挂载（视觉抽动） */
  pid: number
  name: string
  memoryLabel: string
  /** 占物理内存百分比（1~100） */
  percent: number
  /** 原生应用图标 base64（后端按 pid/ppid/name 匹配 bundle 后按需取图，
   * mtime 校验长期缓存）；缺省时走 bundlePath + iconService 预取 */
  icon?: string
  /** 应用 bundle 路径（.app）：iconService 的取图键，与 Uninstall 列表同源 */
  bundlePath?: string
}

/**
 * 内存占用详细区（腾讯柠檬风格）：
 * 标题行右侧「释放」按钮，对接后端 mole_purge_memory（柠檬 QMPurgeRAM 的 Rust
 * 移植：mmap/malloc/zone 三层策略 + 15s 超时 + 10s 频控；反馈口径为释放
 * 前后 vm_stat free 页差值），前端用 formatBytes 直接呈现 MB/GB。
 * 列表 flex-1 填满气泡剩余空间 + 滚动。
 * 释放状态机：idle → running（等后端，按钮 Loading + 禁用）→ done(1.8s) → idle。
 * 图标（对齐 Uninstall 页模式，消除闪烁）：
 *   1. 后端随帧下发的 icon 优先直渲（零 IPC）；
 *   2. 缺省时按 bundlePath 走 iconService 批量预取，完成前先显示骨架屏（AppIcon
 *      已去掉首字母色块），完成后才换成真实图标；
 *   3. 单一图标源 + 稳定 key（pid），不存在「旧图标 → 新图标」跳变；两条路径
 *      最终调用同一个后端编码器，data URI 逐字节相同。
 */
export function MemoryList({ processes, availableGb }: { processes: MemProcess[]; availableGb?: number }) {
  const tauri = useTauri()
  const [phase, setPhase] = useState<ReleasePhase>('idle')
  const [freedLabel, setFreedLabel] = useState('')
  const doneTimer = useRef<number | undefined>(undefined)

  // ── 图标预取（对齐 Uninstall：批量一次 IPC + 就绪后才渲染真实图标）──
  // AppIcon 通过 iconService.getCachedSync() 同步读缓存，故预取完成后需 bump
  // 一次状态触发重渲染。只请求「后端未随帧下发图标 且 尚未缓存 且 不在飞行中」
  // 的路径，因此稳态（每 2s 一帧）下不会再发 IPC，也不会反复闪骨架屏。
  const [, bumpIconReady] = useState(0)
  const inflightRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    const paths: string[] = []
    for (const p of processes) {
      const path = p.bundlePath
      if (!path || p.icon) continue
      if (iconService.getCachedSync(path)) continue
      if (inflightRef.current.has(path)) continue
      paths.push(path)
    }
    if (paths.length === 0) return

    for (const path of paths) inflightRef.current.add(path)
    let cancelled = false
    iconService
      .preloadIcons(paths)
      .then(() => {
        if (!cancelled) bumpIconReady((n) => n + 1)
      })
      .catch((e: unknown) => console.error('[MemoryList] preloadIcons failed:', e))
      .finally(() => {
        for (const path of paths) inflightRef.current.delete(path)
      })

    return () => {
      cancelled = true
    }
  }, [processes])

  // 组件卸载时清理 done 倒计时，避免滞后 setState
  useEffect(() => () => window.clearTimeout(doneTimer.current), [])

  const handleRelease = async () => {
    if (phase !== 'idle') return
    setPhase('running')
    try {
      const res = await tauri.mole_purge_memory()
      if (res?.throttled) {
        // 柠檬 10s 频控（后端节流时已 sleep(1) 防连点空耗）：友好提示并回初始态
        moleMessage.info(`释放太频繁，请 ${res.retry_after_secs ?? 10} 秒后再试`)
        setPhase('idle')
        return
      }
      // 柠檬口径：freed_bytes 为释放前后 free 页差值；0 表示无可释放页
      const freed = res?.freed_bytes ?? 0
      setFreedLabel(freed > 0 ? formatBytes(freed) : '')
      setPhase('done')
      doneTimer.current = window.setTimeout(() => setPhase('idle'), 1800)
    } catch (e) {
      console.error('[MemoryList] mole_purge_memory failed:', e)
      moleMessage.error('释放失败')
      setPhase('idle')
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="mb-1 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-medium" style={{ color: t.textTertiary }}>
            内存占用
          </span>
          {availableGb !== undefined && (
            <span className="text-[11px] tabular-nums" style={{ color: t.textSecondary }}>
              {availableGb.toFixed(1)} GB 可用
            </span>
          )}
        </div>
        <MoleButton
          onClick={handleRelease}
          variant="link"
          color="danger"
          size="medium"
          disabled={phase !== 'idle'}
          styles={{
            root: {
              fontSize: 11,
              fontWeight: 450,
              // 内联色覆盖 antd link-danger/禁用灰：running 保持主题色，done 用反馈绿
              ...(phase === 'running' ? { color: t.textSecondary } : {}),
              ...(phase === 'done' ? { color: t.success } : {})
            }
          }}
          className={`transition-all ${phase === 'idle' ? 'hover:brightness-90' : ''
            } ${phase === 'running' ? 'cursor-wait' : ''}`}
        >
          {phase === 'running' && <Loader2 size={11} className="animate-spin" />}
          {phase === 'done' && <Check size={11} />}
          {phase === 'running' ? '正在释放...' : phase === 'done' ? (freedLabel ? `已释放 ${freedLabel}` : '内存状态良好') : '释放'}
        </MoleButton>
      </div>

      {/* 统一滚动条契约（对齐 Clean/Uninstall 的 SimpleBar 模式）：
          .mole-scroll 空闲隐藏、滚动/悬停显示；-mr-4 抵消父级右内边距使轨道贴气泡右缘，
          内层 pr-4 补偿内容留白；flex-1 min-h-0 + maxHeight:100% 与 UninstallTab 同款。 */}
      <SimpleBar className="mole-scroll -mr-4 min-h-0 flex-1" style={{ maxHeight: '100%' }}>
        <div className="flex flex-col gap-1.5 pr-4">
          {processes.map((p) => {
            // 单一图标源（防跳变）：后端随帧下发的 icon 优先；缺省时才交给
            // iconService（按 bundlePath 同步读缓存，未就绪则 AppIcon 显示骨架屏）。
            const iconSrc = p.icon ? `data:image/png;base64,${p.icon}` : null
            return (
              /* 柠檬 LMMemoryCellView 水位语法：整行即轨道（track 底色），
                 内部按占比叠一层 accent 水位色块，视觉上是「内存水位」而非细线进度条 */
              <div
                key={p.pid}
                className="relative overflow-hidden rounded">
                <div
                  className="absolute inset-y-0 left-0 transition-all duration-500"
                  style={{ width: `${p.percent}%`, background: t.waterFill }}
                />
                <div className="relative flex items-center gap-2.5 p-1">
                  {/* 原生应用图标：后端 icon 直渲，或 iconService 预取后同步命中；
                      两者都未就绪时为骨架屏（animate-pulse），不用首字母色块 */}
                  <AppIcon
                    name={p.name}
                    iconSrc={iconSrc}
                    path={iconSrc ? undefined : p.bundlePath}
                    size={22}
                  />
                  <span className="min-w-0 flex-1 truncate text-[12px]" style={{ color: t.textPrimary }}>
                    {p.name}
                  </span>
                  <span className="shrink-0 text-[11px] tabular-nums" style={{ color: t.textSecondary }}>
                    {p.memoryLabel}
                  </span>
                  <span className="w-8 shrink-0 text-right text-[10px] tabular-nums" style={{ color: t.textTertiary }}>
                    {p.percent}%
                  </span>
                </div>
              </div>
            )
          })}
        </div>
      </SimpleBar>
    </div>
  )
}
