import { activePalette } from './themeColors'
import { useActiveId } from './routing'

/* ── SVG 齿轮路径（从 9-1.html 提取，完全一致） ── */
const GEAR_PATH =
  'M50 10 L54 10 L56 2 L60 2 L62 10 L66 12 L72 6 L76 8 L72 16 L74 20 L82 18 L84 22 L76 26 L76 30 L84 34 L82 38 L74 36 L72 40 L78 46 L74 48 L68 42 L64 44 L66 52 L62 54 L58 46 L54 46 L54 54 L50 54 L48 46 L44 46 L42 54 L38 52 L40 44 L36 42 L30 48 L26 46 L32 40 L30 36 L22 38 L20 34 L28 30 L28 26 L20 22 L22 18 L30 20 L32 16 L28 8 L32 6 L38 12 L42 10 L44 2 L48 2 L50 10Z'

export default function GearBackground() {
  const activeId = useActiveId()
  const theme = activePalette[activeId] ?? activePalette.home

  return (
    <>
      <style>{`
        .hero-illustration {
          position: absolute;
          width: 100%; height: 100%;
          top: 0; left: 0;
          pointer-events: none;
          z-index: 0;
          overflow: hidden;
        }
        .hero-illustration .gear-svg {
          position: absolute;
          opacity: 0.06;
          animation: spinGear 20s linear infinite;
          color: ${theme.accent};
        }
        .hero-illustration .gear-svg:nth-child(1) {
          width: 300px; height: 300px;
          top: -60px; left: -80px;
          animation-duration: 30s;
        }
        .hero-illustration .gear-svg:nth-child(2) {
          width: 200px; height: 200px;
          bottom: -40px; right: -40px;
          animation-duration: 20s;
          animation-direction: reverse;
        }
        .hero-illustration .gear-svg:nth-child(3) {
          width: 150px; height: 150px;
          top: 20%; right: 8%;
          animation-duration: 25s;
          opacity: 0.04;
        }
        @keyframes spinGear {
          0% { transform: rotate(0deg); }
          100% { transform: rotate(360deg); }
        }
      `}</style>

      <div className="hero-illustration">
        <svg className="gear-svg" viewBox="0 0 100 100" aria-hidden="true">
          <path d={GEAR_PATH} fill="currentColor" />
          <circle cx="50" cy="28" r="12" fill="none" stroke="currentColor" strokeWidth="3" />
        </svg>
        <svg className="gear-svg" viewBox="0 0 100 100" aria-hidden="true">
          <path d={GEAR_PATH} fill="currentColor" />
          <circle cx="50" cy="28" r="12" fill="none" stroke="currentColor" strokeWidth="3" />
        </svg>
        <svg className="gear-svg" viewBox="0 0 100 100" aria-hidden="true">
          <path d={GEAR_PATH} fill="currentColor" />
          <circle cx="50" cy="28" r="12" fill="none" stroke="currentColor" strokeWidth="3" />
        </svg>
      </div>
    </>
  )
}
