/**
 * 前端 UI 时序埋点（卡顿分析专用，临时设施）。
 *
 * 用途：把 WebView 侧关键时间轴——页面挂载/卸载、任务快照迁移、扫描事件计数、
 * 渲染帧率、路由点击 → 提交耗时——同时打到 console 与后端 mole.log 的 `[ui:<tag>]` 行，
 * 与 `[clean-job]` / `[section]` 等后端日志落在同一份文件、同一时间轴，
 * 用于定位「清理界面卡顿 / 路由点击不响应几秒」这类跨前后端问题。
 *
 * 注意：必须使用原生 invoke 而非 useTauri 封装——useTauri 的 Batcher 会按
 * cmd+payload 合并并发调用，相同内容的日志会被吞掉。
 */
import { invoke } from '@tauri-apps/api/core'
import { CMD_MOLE_UI_LOG } from '@/constants/tauri-commands'

/** 模块加载时刻作为相对时间原点（与后端日志按绝对时间对齐即可） */
const T0 = performance.now()
let seq = 0

/** 构建形态标记：0=dev（vite），1=构建版（dist）——两版双开对比时用于区分日志来源 */
const BUILD_TAG = import.meta.env.PROD ? 1 : 0

/** 记录一条 UI 时序埋点：console + 转发到后端 `[ui:<tag>]` 日志行；失败静默，不影响业务 */
export function uiTrace(tag: string, message = ''): void {
  const t = ((performance.now() - T0) / 1000).toFixed(2)
  seq += 1
  const line = `t=${t}s n=${seq} p=${BUILD_TAG}${message ? ` ${message}` : ''}`
  console.log(`[ui:${tag}] ${line}`)
  invoke(CMD_MOLE_UI_LOG, { tag, message: line }).catch(() => { /* 埋点失败不影响业务 */ })
}

// ---- 导航点击 → 路由提交 的跨组件标记 ----
// layout/index.tsx 的 handleNavigate 写入，ShellContent 在 pathname 提交时取走，
// 用于计算「用户点击路由」到「新页面真正渲染」的全链路延迟。
let navClickMark: { id: string; at: number } | null = null

/** 记录一次导航点击（写入标记 + 打出 nav.click 日志） */
export function markNavClick(id: string): void {
  navClickMark = { id, at: performance.now() }
  uiTrace('nav.click', `id=${id}`)
}

/** 取走最近一次导航点击标记（一次性消费） */
export function takeNavClickMark(): { id: string; at: number } | null {
  const m = navClickMark
  navClickMark = null
  return m
}

// ---- 路由 render 起点跟踪（拆「点击→开始渲染」与「渲染→提交」两段耗时）----
// ShellContent 在 render 体里 markRender；effect 里 peekRenderAt 取回。
// 只在 pathname 首次出现时记时（StrictMode 双渲染 / 同路径重渲染不覆盖）。
let renderMark: { path: string; at: number } | null = null

/** 记录某个 pathname 的首次 render 时刻（组件 render 体调用） */
export function markRender(path: string): void {
  if (renderMark?.path !== path) renderMark = { path, at: performance.now() }
}

/** 取回某个 pathname 的首次 render 时刻（不存在返回 null） */
export function peekRenderAt(path: string): number | null {
  return renderMark?.path === path ? renderMark.at : null
}

// ---- 主线程阻塞监测（切页卡顿定位用）----
// WKWebView 不支持 PerformanceObserver longtask，用 rAF 帧间隔近似：
// 相邻帧间隔 > 120ms 说明主线程被长任务占住（仅在页面可见时计入；
// 从隐藏/睡眠恢复时通过 visibilitychange 重置基线，避免误报）。
let stallWatchStarted = false

/** 启动 rAF 阻塞监测（模块加载时自动调用一次） */
function startStallWatch(): void {
  if (stallWatchStarted || typeof window === 'undefined') return
  stallWatchStarted = true
  let last = performance.now()
  document.addEventListener('visibilitychange', () => { last = performance.now() })
  const loop = () => {
    const now = performance.now()
    const gap = now - last
    last = now
    if (gap > 120 && document.visibilityState === 'visible') {
      uiTrace('stall', `gap=${gap.toFixed(0)}ms`)
    }
    requestAnimationFrame(loop)
  }
  requestAnimationFrame(loop)
}

startStallWatch()
