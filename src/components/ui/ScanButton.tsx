import {MoleButton, MoleButtonProps } from './MoleButton'
import './ScanButton.scss'

export interface ScanButtonProps extends MoleButtonProps {
  /** 启用扫描光晕动画：hover 上浮 + active 下压 + ::after 白色光晕扩散（默认开启） */
  scanEffect?: boolean
}

export function ScanButton({ className = '', scanEffect = true, ...rest }: ScanButtonProps) {
  return (
    <MoleButton
      className={`${scanEffect ? 'mole-scan-btn' : ''} ${className}`.trim()}
      {...rest}
    />
  )
}
