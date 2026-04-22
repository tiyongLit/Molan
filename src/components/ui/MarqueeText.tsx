import { useEffect, useLayoutEffect, useRef, useState } from 'react'

// ── 跑马灯参数 — 严格对齐 Lemon（MPScrollingTextField 默认值 + 气泡覆盖配置）：
//    scrollingRate=20（commonInit 默认，气泡未覆盖）/ scrollingOffset=10（气泡显式设置）/
//    首滚前停 1s（updateLayerFrames 内 dispatch_after 1s）/
//    每程后停 2s（animationDidStop 1s + updateLayerFrames 1s 两段串联）──
const GAP = 10 // 两份文本间距（px）— Lemon scrollingOffset（气泡显式设置）
const RATE = 20 // 滚动速度 px/s — Lemon scrollingRate（commonInit 默认）
const INITIAL_HOLD_MS = 150 // 首次滚动前停留（缩短到 150ms，悬停后快速响应）
const HOLD_MS = 1500 // 单程结束后停留（缩短到 1.5s，循环更紧凑）
const FADE_W = 20 // 边缘渐隐宽度（px）— 对齐原覆盖层 w-5

interface MarqueeTextProps {
  /** 纯文本内容（与 children 二选一，气泡悬停场景沿用） */
  text?: string
  /** 富资源（ReactNode，可含高亮文本/分隔符等组合信息）；提供时优先于 text */
  children?: React.ReactNode
  className?: string
  /**
   * 受控激活（如父级按钮 hover）：
   * - 缺省：超宽即自动滚动（底栏统计行 / 气泡用法，行为不变）
   * - false：不滚动，超宽时以 CSS 省略号截断（静态展示）
   * - true：超宽时滚动展示完整内容；未超宽则静态
   */
  active?: boolean
}

/**
 * 边缘渐隐 mask — 替代原「底色覆盖层」方案：
 * 不依赖具体底色，气泡（纯色底）与透明浮层（渐变背景）场景通用。
 * 静态超宽仅右缘淡出；滚动中左右两缘同时淡出（对齐 Lemon mask 三态）。
 */
function fadeMask(scrolling: boolean): React.CSSProperties {
  const gradient = scrolling
    ? `linear-gradient(90deg, transparent 0, #000 ${FADE_W}px, #000 calc(100% - ${FADE_W}px), transparent 100%)`
    : `linear-gradient(90deg, #000 calc(100% - ${FADE_W}px), transparent 100%)`
  return { maskImage: gradient, WebkitMaskImage: gradient }
}

/**
 * 跑马灯文本 — 逐项对齐 Lemon Cleaner MPScrollingTextField 行为：
 * - 未超宽：静态显示，无渐隐
 * - 超宽：双份复制 + 单程线性滚动（0 → -(文本宽+间距)），到位后瞬间复位
 *   （第二份文本恰好补到第一份原位，复位瞬间像素对齐、视觉无缝）
 * - 时序：首滚前停 1s → 滚动一程 → 停 2s → 循环往复（无限轮播，直到组件卸载）
 * - 渐隐：mask-image 边缘淡出（静态超宽仅右缘；滚动中左右两缘）
 * - 内容：text（纯文本）或 children（富内容）二选一
 * - 激活：缺省超宽即滚（自动）；传 active 则受控（false 静态省略号 / true 滚动），
 *   适配「悬停时才轮播」场景（面包屑段、卡片 footnote）
 */
export function MarqueeText({ text, children, className, active }: MarqueeTextProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const trackRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLSpanElement>(null)
  const [textWidth, setTextWidth] = useState(0)
  const [overflowing, setOverflowing] = useState(false)
  const [scrolling, setScrolling] = useState(false)

  const controlled = active !== undefined
  // 滚动条件：超宽 &&（自动模式 || 受控激活）
  const marqueeOn = overflowing && (controlled ? active === true : true)
  // 受控未激活：静态 + CSS 省略号截断（未超宽时省略号类无视觉效果）
  const staticEllipsis = controlled && !marqueeOn

  // 挂载/内容变化时测量：内容完整宽度（scrollWidth 含被截断部分）vs 容器可用宽度
  useLayoutEffect(() => {
    const container = containerRef.current
    const content = contentRef.current
    if (!container || !content) return
    const w = content.scrollWidth
    setTextWidth(w)
    setOverflowing(w > container.clientWidth)
  }, [text, children])

  // 激活且超宽时启动滚动循环（Web Animations API，无需全局 keyframes）
  useEffect(() => {
    if (!marqueeOn || textWidth <= 0) return
    const track = trackRef.current
    if (!track) return

    let stopped = false
    let timer: number | undefined
    let anim: Animation | undefined

    // Lemon 公式：duration = (stringWidth + scrollingOffset) / scrollingRate
    const distance = textWidth + GAP
    const duration = (distance / RATE) * 1000

    const runPass = () => {
      if (stopped) return
      setScrolling(true)
      anim = track.animate(
        [
          { transform: 'translateX(0)' },
          { transform: `translateX(-${distance}px)` }
        ],
        { duration, easing: 'linear', fill: 'forwards' }
      )
      anim.onfinish = () => {
        setScrolling(false)
        anim?.cancel() // 移除 fill 效果 → 瞬间复位到 0；第二份文本恰好补位，视觉无缝
        if (!stopped) timer = window.setTimeout(runPass, HOLD_MS)
      }
    }

    // 首次滚动前先停 1s（对齐 Lemon updateLayerFrames 的 dispatch_after 1s）
    timer = window.setTimeout(runPass, INITIAL_HOLD_MS)
    return () => {
      stopped = true
      if (timer !== undefined) window.clearTimeout(timer)
      anim?.cancel()
      setScrolling(false)
    }
  }, [marqueeOn, textWidth])

  const content = children ?? text

  return (
    <div
      ref={containerRef}
      className={`relative overflow-hidden ${className ?? ''}`}
      style={marqueeOn ? fadeMask(scrolling) : undefined}
    >
      <div ref={trackRef} className="flex items-center">
        <span
          ref={contentRef}
          className={staticEllipsis ? 'min-w-0 flex-1 truncate' : 'shrink-0 whitespace-nowrap'}
        >
          {content}
        </span>
        {marqueeOn && (
          <span className="shrink-0 whitespace-nowrap" style={{ paddingLeft: GAP }} aria-hidden>
            {content}
          </span>
        )}
      </div>
    </div>
  )
}
