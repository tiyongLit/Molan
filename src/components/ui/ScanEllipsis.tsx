import './ScanEllipsis.css'

/**
 * 循环省略号（...）：扫描/优化状态条的等待指示，
 * 三点依次淡入淡出，形成实时变化的扫描动效。
 */
export function ScanEllipsis() {
  return (
    <span className="scan-status__ellipsis" aria-hidden>
      <span>.</span>
      <span>.</span>
      <span>.</span>
    </span>
  )
}
