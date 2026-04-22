import { useEffect, useMemo } from 'react'
import { AuthBanner } from './AuthBanner'
import { BottomBar } from './BottomBar'
import { formatBytes } from './format'
import { MemoryList, type MemProcess } from './MemoryList'
import { NetworkCard } from './MetricCard'
import { StatusBar } from './StatusBar'
import { dashTheme } from './theme'
import { useStatusSnapshot } from './useStatusSnapshot'
import { useSettings } from '@/pages/Settings/useSettings'
import { deriveDiskMetrics, pickPrimaryDisk } from '@/utils/platform'
import { SIZE_BASE } from '@/constants/shared'
import { useI18n } from '@/i18n'

/**
 * 内存单位基数：macOS 「关于本机」的内存显示使用二进制（GiB），
 * 但标签写 GB（如 16 GB = 16 × 1024³ 字节）。
 * SIZE_BASE = 1000 仅用于磁盘（macOS 储存概述用十进制）。
 */
const MEM_SIZE_BASE = 1024 as const

/**
 * V2 系统托盘仪表盘（真实数据阶段）。
 *
 * 布局（自上而下，CleanMyMac 卡片语法 + Burrow 卡片维度）：
 *   授权提示区 → Mac 概览标题 → 2×3 五卡（CPU/内存/磁盘/网络/风扇）
 *   → 内存占用详细区（柠檬风格 + 释放）→ 底部工具栏。
 *
 * 数据源：controllers/status.rs 每秒一帧的 status::snapshot（watch 生命周期由 tray.rs 管理）。
 * 首帧到达前卡片显示占位（0/–），内存列表保持空——不使用 Mock 进程，避免首帧出现假数据/首字母色块。
 */
export function Dashboard() {
  const { snap, hist } = useStatusSnapshot()
  const { settings } = useSettings()
  const { t } = useI18n()

  // 托盘气泡：透明窗口需要 body / #root 背景透明（对齐 MainLayout 的处理）
  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.body.style.backgroundColor = 'transparent'
    const root = document.getElementById('root')
    if (root) root.style.background = 'transparent'
  }, [])

  // ── 核心视图数据 ─────────────────────────────────────────────
  const cpuTemp = snap ? Math.round(snap.thermal?.cpu_temp || 0) : 0
  const mem = snap?.memory
  // 内存首帧前显示 "—" 占位（对齐磁盘卡），不使用 Mock 假数据
  const availableGb = mem ? mem.available / MEM_SIZE_BASE ** 3 : undefined

  const disk = useMemo(() => {
    // 与 Home/Analyze 同源同口径：同一 status::snapshot 事件流 + deriveDiskMetrics 统一派生。
    // 不复用 useDiskStatus 钩子本体：它会触发单次全量采集（mole_status_once），而托盘窗常驻
    // （hidden 预加载），watch 生命周期由 tray.rs 引用计数管理，此处只 listen（见 useStatusSnapshot 注释）。
    const d = pickPrimaryDisk(snap?.disks)
    if (!d) return null // 首帧 Full 采集期间不兜底 MOCK——110.29 GB 与真实值偏差巨大
    const m = deriveDiskMetrics(d)
    return {
      name: m.name,
      // SIZE_BASE=1000（十进制），与 humanDiskSize()/formatSize() 及 macOS 储存概述一致
      freeGb: m.free / SIZE_BASE ** 3,
      totalGb: m.total / SIZE_BASE ** 3,
      usedPercent: Math.round(m.usedPercent),
    }
  }, [snap])

  // 使用 network_history 聚合值（所有物理接口 sum），与 sparkline history 同源。
  // 避免 network[0]（Top 1 单接口）在接口切换时跳变、与 history 末端对不上。
  const net = useMemo(() => {
    const h = snap?.network_history
    if (!h) return { downMBs: 0, upMBs: 0 }
    return { downMBs: h.rx_latest, upMBs: h.tx_latest }
  }, [snap])

  const netHistRx = hist.netRx.length >= 2 ? hist.netRx : []
  const netHistTx = hist.netTx.length >= 2 ? hist.netTx : []

  // ── 内存占用列表（后端 top_processes 已完成 Regular 过滤 + 聚合 RSS 排序 + 截断） ──
  const processes: MemProcess[] = useMemo(() => {
    const total = snap?.memory?.total
    if (!snap?.top_processes?.length || !total) return []
    // 后端是单一事实源（已排序 + 截断 MEMORY_LIST_LIMIT=20），前端只按帧渲染，
    // 不再二次 sort/slice（避免与后端 limit 打架、重复排序）。
    return snap.top_processes.map((p) => {
      const bytes = p.memory_bytes ?? 0
      return {
        // 稳定行标识：MemoryList 以 pid 作 React key（旧 key 含 memoryLabel，
        // 每帧变化会导致整行重建：水位条动画重启、图标重新挂载）
        pid: p.pid,
        name: p.name,
        memoryLabel: formatBytes(bytes),
        percent: Math.min(100, Math.max(1, Math.round((bytes / total) * 100))),
        // 进程域图标：后端按 pid 匹配 NSWorkspace runningApplications 索引得到
        // bundle 路径后按需取图（mtime 校验长期缓存）；缺省时前端用 bundlePath
        // 走 nativeIconRegistry 预取 + 骨架屏（对齐 Uninstall 页模式）
        icon: p.icon,
        bundlePath: p.bundle_path
      }
    })
  }, [snap])

  return (
    <div
      // rounded-[10px] 与原生 tray.rs 的 setCornerRadius:10.0 及主窗口系统圆角(≈10px)对齐；
      // 前端 CSS 圆角必须 ≤ 原生裁剪圆角，否则可见圆角由前端这层决定、看起来比主窗口更圆。
      className="flex h-full w-full flex-col overflow-hidden rounded-[10px]"
      style={{ background: dashTheme.pageBg }}
    >
      <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-hidden px-3.5 pb-2 pt-3">
        <AuthBanner />

        <span className="text-[15px] font-bold tracking-tight text-white">{t('dashboard.overview')}</span>

        {/* 顶部状态条：CPU 温度 / 风扇转速 / 磁盘可用 */}
        <StatusBar
          cpuTemp={cpuTemp}
          fanSpeed={snap?.thermal?.fan_speed}
          diskFreeGb={disk?.freeGb}
          diskTotalGb={disk?.totalGb}
        />

        {/* 网络卡片（独占一行） */}
        <NetworkCard downMBs={net.downMBs} upMBs={net.upMBs} downHist={netHistRx} upHist={netHistTx} />

        <MemoryList processes={processes} availableGb={availableGb} enableKillProcess={settings.dashboard.enableKillProcess} />
      </div>

      <BottomBar />
    </div>
  )
}
