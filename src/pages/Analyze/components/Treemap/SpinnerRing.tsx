/** 纯 CSS 旋转环 — 无百分比数据时使用，避免 antd Progress percent=0 显示空白灰圈 */
export function SpinnerRing({ size = 64 }: { size?: number }) {
  return (
    <svg
      className="animate-spin"
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <circle cx="32" cy="32" r="26" stroke="rgba(255,255,255,0.10)" strokeWidth="6" fill="none" />
      <path
        d="M32 6a26 26 0 0 1 26 26"
        stroke="var(--orange-400)"
        strokeWidth="6"
        strokeLinecap="round"
      />
    </svg>
  )
}
