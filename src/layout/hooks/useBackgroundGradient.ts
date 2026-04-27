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

export function useBackgroundGradient(activeId: string) {
  const palette = activePalette[activeId] ?? activePalette.home
  const isFirstRef = useRef(true)

  const bloom = useRgb(palette.bloom)
  const deep = useRgb(palette.deep)

  const bgGradient = useMotionTemplate`
    linear-gradient(160deg,
      rgb(${bloom[0]}, ${bloom[1]}, ${bloom[2]}) 0%,
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
    animateRgb(deep, next.deep)
  }, [activeId])

  return bgGradient
}
