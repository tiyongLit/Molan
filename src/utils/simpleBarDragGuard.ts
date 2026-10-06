/**
 * SimpleBar 拖拽失联看门狗（全站共享：main.tsx 安装一次，覆盖所有窗口）。
 *
 * 背景（simplebar-core@1.3.2, dist/index.mjs）：
 * 拖拽开始时库会做两件全局性动作——给根元素加 `simplebar-dragging` 类
 * （CSS 使整个子树 pointer-events:none），并在 document 上注册
 * mousemove/mouseup 拖拽监听与 click/dblclick 捕获拦截（preventClick）；
 * 而复位只有一个入口：document 收到 mouseup 时执行 onEndDrag
 * （移除类 / 注销监听 / 延时清理 preventClick）。一旦这次 mouseup 丢失
 * （拖拽中切换应用、系统手势打断、指针移出窗口释放等），滞留态会让
 * 列表区点击/悬停/滚轮全部失效、全应用点击被 preventClick 吞掉、
 * 松键后内容仍跟随鼠标——表现为"UI 假死"。
 *
 * 对策：在 document 捕获阶段轻量巡检 mousemove / wheel——仅当"未按任何
 * 键"（真实拖拽中 buttons !== 0，不会误判）且节流窗口到期时查询
 * `[data-simplebar].simplebar-dragging`；命中即向 document 派发一次合成
 * mouseup，借库自身的 onEndDrag 完成正规复位（含 preventClick 清理）；
 * 派发后类仍残留（极端情形：监听器缺失）则兜底手动移除，至少恢复
 * pointer-events。
 *
 * 依赖约束：兜底依赖 simplebar 以 mouseup 作为拖拽复位信号；
 * 未来升级 simplebar 后若其改听 pointerup，需同步调整此处。
 */

let installed = false

/** 巡检节流窗口：指针移动/滚轮期间的查询频率上限（4 次/秒，成本可忽略） */
const RECOVER_THROTTLE_MS = 250

export function installSimpleBarDragGuard(): void {
  if (installed) return
  installed = true

  let lastCheckAt = 0

  const recover = () => {
    const stuck = document.querySelector<HTMLElement>('[data-simplebar].simplebar-dragging')
    if (!stuck) return
    // 合成 mouseup：命中 SimpleBar 注册在 document 捕获阶段的 onEndDrag
    document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true }))
    // 兜底：极端情形（监听器缺失）下至少恢复 pointer-events
    stuck.classList.remove('simplebar-dragging')
  }

  const inspect = (e: MouseEvent) => {
    // 有按键按下 = 真实拖拽进行中（或用户正在按压），不打扰
    if (e.buttons !== 0) return
    const now = Date.now()
    if (now - lastCheckAt < RECOVER_THROTTLE_MS) return
    lastCheckAt = now
    recover()
  }

  // 失联后：用户一动鼠标（或第一次尝试滚动）即刻恢复
  document.addEventListener('mousemove', inspect, true)
  document.addEventListener('wheel', inspect, { capture: true, passive: true })
}
