import { ProgressDial } from '@/components/ProgressDial'
import { useScanButton } from './ScanButtonContext'

interface CircleButtonProps {
  accent: string
  bloom: string
}

export function CircleButton({ accent, bloom }: CircleButtonProps) {
  const { state, triggerOnClick } = useScanButton()
  if (!state.visible) return null

  const isIdle = state.percent === 0
  const glowColor = `rgba(${bloom}, 0.25)`
  const ringColor = isIdle ? (state.strokeColor || accent) : accent

  return (
    <div className="shell-circle-btn z-10" onClick={triggerOnClick}>
      <ProgressDial
        percent={state.percent}
        size={88}
        strokeWidth={6}
        strokeColor={ringColor}
        glowColor={glowColor}
      >
        <div className="flex flex-col items-center justify-center leading-none">
          {isIdle ? (
            <span className="text-sm font-semibold" style={{ color: '#e8faf3' }}>
              {state.label}
            </span>
          ) : (
            <>
              <span className="text-lg font-bold" style={{ color: '#e8faf3' }}>
                {state.percent}%
              </span>
              <span className="text-[10px] mt-0.5" style={{ color: '#a0d4c0' }}>
                {state.label}
              </span>
            </>
          )}
        </div>
      </ProgressDial>
    </div>
  )
}
