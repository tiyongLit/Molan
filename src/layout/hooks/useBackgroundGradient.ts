import { useEffect, useRef } from 'react'
import { useMotionValue, useMotionTemplate, animate } from 'motion/react'
import type { MotionValue } from 'motion/react'
import { activePalette, type RGB } from '../themeColors'

function useRgb(initial: RGB) {
  const r = useMotionValue(initial[0])
  const g = useMotionValue(initial[1])
  const b = useMotionValue(initial[2])
  return [r, g, b] as const
}

function animateRgb(vals: readonly MotionValue<number>[], target: RGB) {
  vals.forEach((v, i) => animate(v, target[i], { duration: 0.5, ease: 'easeInOut' }))
}

// 两端 45% 插值点。三段 stop 共线于 bloom→deep 直线（两段斜率均 (deep-bloom)/100），
// 因此缺省 mid 时三段模板与两段线性渐变视觉完全一致，已启用页面不受影响。
function mixRgb45(a: RGB, b: RGB): RGB {
  return [
    Math.round(a[0] + (b[0] - a[0]) * 0.45),
    Math.round(a[1] + (b[1] - a[1]) * 0.45),
    Math.round(a[2] + (b[2] - a[2]) * 0.45),
  ]
}

export function useBackgroundGradient(activeId: string) {
  const palette = activePalette[activeId] ?? activePalette.home
  const isFirstRef = useRef(true)

  const bloom = useRgb(palette.bloom)
  const mid = useRgb(palette.mid ?? mixRgb45(palette.bloom, palette.deep))
  const deep = useRgb(palette.deep)

  const bgGradient = useMotionTemplate`
    linear-gradient(160deg,
      rgb(${bloom[0]}, ${bloom[1]}, ${bloom[2]}) 0%,
      rgb(${mid[0]}, ${mid[1]}, ${mid[2]}) 45%,
      rgb(${deep[0]}, ${deep[1]}, ${deep[2]}) 100%
    )
  `

  useEffect(() => {
    if (isFirstRef.current) {
      isFirstRef.current = false
      return
    }

    const next = activePalette[activeId] ?? activePalette.home
    animateRgb(bloom, next.bloom)
    animateRgb(mid, next.mid ?? mixRgb45(next.bloom, next.deep))
    animateRgb(deep, next.deep)
  }, [activeId])

  return bgGradient
}
