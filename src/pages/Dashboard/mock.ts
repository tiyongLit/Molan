// V2 仪表盘兜底数据：真实数据接入后，本文件仅作为首帧到达前的占位
// 与浏览器预览（无 IPC）的降级渲染；快照字段见 useStatusSnapshot。

export interface MockProcess {
  name: string
  memoryLabel: string
  percent: number
  /** App bundle 路径：交给 AppIcon 命中原生图标（与 Uninstall 同源），未命中回退首字母色块 */
  path: string
}

export const MOCK = {
  cpu: { usage: 19, temp: 56 },
  memory: { usedPercent: 62, availableGb: 4.8, totalGb: 16 },
  // disk 已从 MOCK 移除：磁盘数据必须由后端快照下发（diskFreeBytes/pickPrimaryDisk），
  // 首帧采集期间 DiskCard 显示 "—" 占位，绝不回退到与真实值偏差巨大的假数据。
  network: { down: 492, up: 128, unit: 'KB/s', ssid: 'Wi-Fi' },
}

// 趋势序列（静态波形，纯视觉演示；接入后由前端 ring buffer 60 点提供）
export const CPU_HIST = [12, 14, 11, 16, 22, 34, 28, 19, 17, 21, 26, 31, 24, 18, 22, 19]
export const MEM_HIST = [58, 59, 60, 60, 61, 62, 61, 62, 63, 62, 62, 61, 62, 62, 63, 62]
export const NET_HIST = [20, 35, 28, 60, 82, 55, 40, 66, 90, 48, 30, 52, 70, 44, 36, 58]

export const MOCK_PROCESSES: MockProcess[] = [
  { name: 'Chrome', memoryLabel: '1.2 GB', percent: 32, path: '/Applications/Google Chrome.app' },
  { name: 'Xcode', memoryLabel: '856 MB', percent: 22, path: '/Applications/Xcode.app' },
  { name: 'Safari', memoryLabel: '420 MB', percent: 11, path: '/Applications/Safari.app' },
  { name: 'Slack', memoryLabel: '312 MB', percent: 8, path: '/Applications/Slack.app' },
  { name: 'Finder', memoryLabel: '180 MB', percent: 5, path: '/System/Library/CoreServices/Finder.app' },
  { name: 'Terminal', memoryLabel: '96 MB', percent: 3, path: '/System/Applications/Utilities/Terminal.app' },
  { name: 'Music', memoryLabel: '72 MB', percent: 2, path: '/System/Applications/Music.app' },
]
