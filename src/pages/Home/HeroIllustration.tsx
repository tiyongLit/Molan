// ── Hero 插图：磁盘核心 ──
// 双层虚线环模拟 HDD 盘片旋转，内部呼吸脉冲，外部粒子微浮动
// viewBox 280×240
// 颜色统一为 accent 色系，由容器 opacity 统一控制透明度，不再内部衰减

const PI = Math.PI

function polarToCartesian(cx: number, cy: number, r: number, angleDeg: number) {
  const rad = ((angleDeg - 90) * PI) / 180
  return { x: cx + r * Math.cos(rad), y: cy + r * Math.sin(rad) }
}

function sectorPath(
  cx: number, cy: number,
  r: number,
  startDeg: number, endDeg: number,
) {
  const s = polarToCartesian(cx, cy, r, endDeg)
  const e = polarToCartesian(cx, cy, r, startDeg)
  const large = endDeg - startDeg > 180 ? 1 : 0
  return [
    `M ${cx} ${cy}`,
    `L ${s.x} ${s.y}`,
    `A ${r} ${r} 0 ${large} 0 ${e.x} ${e.y}`,
    'Z',
  ].join(' ')
}

export default function HeroIllustration({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 280 240"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
    >
      <style>{`
        @keyframes ring-rotate-cw {
          from { transform: rotate(0deg); }
          to   { transform: rotate(360deg); }
        }
        @keyframes ring-rotate-ccw {
          from { transform: rotate(360deg); }
          to   { transform: rotate(0deg); }
        }
        .ring-outer-anim {
          transform-origin: 140px 120px;
          animation: ring-rotate-cw 20s linear infinite;
        }
        .ring-inner-anim {
          transform-origin: 140px 120px;
          animation: ring-rotate-ccw 15s linear infinite;
        }

        @keyframes pulse-core {
          0%, 100% { opacity: 0.6; }
          50%      { opacity: 1; }
        }
        .pulse-core {
          animation: pulse-core 2.5s ease-in-out infinite;
        }

        @keyframes pulse-inner {
          0%, 100% { opacity: 0.4; }
          50%      { opacity: 1; }
        }
        .pulse-inner {
          animation: pulse-inner 2s ease-in-out infinite;
        }

        @keyframes float-up {
          0%, 100% { transform: translateY(0); opacity: 0.5; }
          50%      { transform: translateY(-2px); opacity: 1; }
        }
        .float-particle {
          animation: float-up 3s ease-in-out infinite;
        }
        .float-particle-delay1 { animation-delay: 0.6s; }
        .float-particle-delay2 { animation-delay: 1.4s; }
        .float-particle-delay3 { animation-delay: 2.1s; }
      `}</style>

      <defs>
        <linearGradient id="ring-outer" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#A0ECD8" />
          <stop offset="100%" stopColor="#8AE0C8" />
        </linearGradient>

        <linearGradient id="ring-inner" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#A0ECD8" />
          <stop offset="100%" stopColor="#7EE0C8" />
        </linearGradient>

        <linearGradient id="core-fill" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#8AE0C8" />
          <stop offset="100%" stopColor="#7EE0C8" />
        </linearGradient>

        <linearGradient id="clean-zone" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="#8AE0C8" stopOpacity="0" />
          <stop offset="60%" stopColor="#8AE0C8" stopOpacity="0.4" />
          <stop offset="100%" stopColor="#A0ECD8" stopOpacity="0.6" />
        </linearGradient>

        <filter id="glow-subtle">
          <feGaussianBlur stdDeviation="2" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {/* ═══ 磁盘核心结构 ═══ */}

      <circle cx="140" cy="120" r="76" fill="none" stroke="#7EE0C8" strokeWidth="0.6" />

      <circle
        cx="140" cy="120" r="68"
        fill="none" stroke="url(#ring-outer)" strokeWidth="1.5"
        strokeDasharray="80 24 40 16"
        className="ring-outer-anim"
      />

      <circle
        cx="140" cy="120" r="56"
        fill="none" stroke="url(#ring-inner)" strokeWidth="1.2"
        strokeDasharray="30 10 18 8"
        filter="url(#glow-subtle)"
        className="ring-inner-anim"
      />

      <path
        d={sectorPath(140, 120, 80, 315, 45)}
        fill="url(#clean-zone)"
      />

      <circle cx="140" cy="120" r="36" fill="url(#core-fill)" className="pulse-core" />
      <circle cx="140" cy="120" r="22" fill="#3f3a5a" />
      <circle cx="140" cy="120" r="10" fill="#8AE0C8" className="pulse-inner" />
      <circle cx="140" cy="120" r="4" fill="#A0ECD8" />

      {/* ═══ 左侧：文件簇（待清理）═══ */}

      <g>
        <rect x="54" y="150" width="7" height="7" rx="1.5" fill="#8AE0C8" />
        <rect x="68" y="156" width="10" height="10" rx="2" fill="#7EE0C8" />
        <rect x="55" y="168" width="14" height="8" rx="2" fill="#A0ECD8" />
        <rect x="75" y="172" width="8" height="8" rx="1.5" fill="#8AE0C8" />
        <circle cx="64" cy="190" r="4" fill="#A0ECD8" />

        <rect x="64" y="50" width="10" height="10" rx="2" fill="#8AE0C8" />
        <rect x="78" y="58" width="8" height="8" rx="1.5" fill="#A0ECD8" />
        <circle cx="56" cy="66" r="4" fill="#7EE0C8" />
        <circle cx="82" cy="46" r="3" fill="#8AE0C8" />
      </g>

      {/* ═══ 右侧：稀疏残余粒子 ═══ */}

      <circle cx="210" cy="95" r="2.5" fill="#A0ECD8" />
      <circle cx="220" cy="130" r="3" fill="#8AE0C8" />
      <circle cx="205" cy="160" r="2" fill="#A0ECD8" />
      <circle cx="228" cy="148" r="1.5" fill="#8AE0C8" />

      {/* ═══ 核心结构上的微粒子点缀 ═══ */}

      <polygon points="190,50.8 191.2,52 190,53.2 188.8,52" fill="none" stroke="#A0ECD8" strokeWidth="0.5" />
      <circle cx="185" cy="204" r="1" fill="#8AE0C8" />
      <polygon points="68,199 69,200 68,201 67,200" fill="#A0ECD8" />
      <circle cx="68" cy="44" r="0.8" fill="#8AE0C8" />

      <polygon points="88,66.8 89.2,68 88,69.2 86.8,68" fill="none" stroke="#A0ECD8" strokeWidth="0.4" />
      <polygon points="202,77 203,78 202,79 201,78" fill="#A0ECD8" />
      <circle cx="80" cy="170" r="0.8" fill="none" stroke="#8AE0C8" strokeWidth="0.4" />

      <circle cx="98" cy="80" r="0.8" fill="#A0ECD8" />
      <polygon points="192,89 193,90 192,91 191,90" fill="none" stroke="#8AE0C8" strokeWidth="0.4" />
      <polygon points="100,161 101,162 100,163 99,162" fill="#A0ECD8" />

      <circle cx="176" cy="141" r="1.2" fill="none" stroke="#A0ECD8" strokeWidth="0.5" />
      <polygon points="182,134 183,135 182,136 181,135" fill="#8AE0C8" />

      <circle cx="198" cy="92" r="1.5" fill="none" stroke="#A0ECD8" strokeWidth="0.5" />
      <polygon points="192,104 193,105 192,106 191,105" fill="#8AE0C8" />
      <circle cx="205" cy="120" r="1" fill="#A0ECD8" />

      <circle cx="42" cy="128" r="1" fill="#8AE0C8" className="float-particle" />
      <polygon points="150,209 151,210 150,211 149,210" fill="#7EE0C8" className="float-particle float-particle-delay1" />
      <circle cx="95" cy="35" r="0.8" fill="#A0ECD8" className="float-particle float-particle-delay2" />
    </svg>
  )
}
