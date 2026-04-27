import type { CSSProperties, ReactNode } from 'react'
import { Result } from 'antd'
import { MoleButton } from './MoleButton'

export interface ResultViewProps {
  /** antd Result 语义状态（决定图标：对勾 / 感叹 / 叉） */
  status: 'success' | 'warning' | 'error'
  /** 主标题 */
  title: ReactNode
  /** 副标题（可选） */
  subtitle?: ReactNode
  /** 次按钮文案（左侧，默认样式） */
  secondaryLabel: string
  /** 次按钮点击回调 */
  onSecondary: () => void
  /** 主按钮文案（右侧，primary 样式） */
  primaryLabel: string
  /** 主按钮点击回调 */
  onPrimary: () => void
  /** 外层容器内联样式（如页面主题变量 PAGE_THEME_VARS） */
  style?: CSSProperties
}

/**
 * 通用结果页：居中 antd Result + 两个操作按钮。
 *
 * Clean ScanResult 与 Optimize OptimizeResult 的呈现层（antd Result 样板、
 * 按钮样式、居中容器）完全一致，仅标题/副标题/按钮文案不同，故提取为
 * 通用组件；状态与文案计算仍由各页薄包装层完成。
 *
 * @example
 * <ResultView
 *   status="success"
 *   title="清理完成"
 *   subtitle="共释放 1.2 GB"
 *   secondaryLabel="重新扫描"
 *   onSecondary={onRescan}
 *   primaryLabel="完成"
 *   onPrimary={onFinish}
 *   style={PAGE_THEME_VARS}
 * />
 */
export function ResultView({
  status,
  title,
  subtitle,
  secondaryLabel,
  onSecondary,
  primaryLabel,
  onPrimary,
  style,
}: ResultViewProps) {
  return (
    <div className="flex select-none h-full w-full items-center justify-center" style={style}>
      <Result
        status={status}
        title={<span className="text-white">{title}</span>}
        subTitle={subtitle ? <span className="text-white/60">{subtitle}</span> : undefined}
        extra={[
          <MoleButton key="secondary" size="large" onClick={onSecondary} style={{ width: 160, height: 50, borderRadius: 12 }}>
            {secondaryLabel}
          </MoleButton>,
          <MoleButton key="primary" type="primary" size="large" onClick={onPrimary} style={{ width: 160, height: 50, borderRadius: 12 }}>
            {primaryLabel}
          </MoleButton>,
        ]}
      />
    </div>
  )
}