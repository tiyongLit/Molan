import type { CSSProperties, ReactNode } from 'react'
import { Result } from 'antd'
import { MoleButton } from './MoleButton'
import './ResultView.scss'

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
  /** 结果区下方的自定义内容（如失败明细卡片，仅失败时由调用方传入） */
  children?: ReactNode
}

// 状态 → 自有徽标资源（public 下，视口 80×80 与圆盘满格重合、无透明边距、无内嵌阴影）。
// warning 暂无专属资源，复用 error 徽标；将来设计出 warning.svg 只需改此映射。
// 渲染 72px 与 antd 内置状态圆同尺寸基准（圆盘=视口，所见即所得）。
// 消费端 img 须用 block + mx-auto：antd icon 槽仅靠 text-align:center 居中
// （只约束 inline 元素），block 消除 baseline 底隙后需 margin auto 补回水平居中。
const STATUS_ICON: Record<ResultViewProps['status'], string> = {
  success: '/success.svg',
  warning: '/error.svg',
  error: '/error.svg',
}

/**
 * 通用结果页：居中 antd Result（徽标走自有 SVG 资源）+ 两个操作按钮。
 * 按钮配色见 ./ResultView.scss（主=绿渐变 CTA / 次=深色玻璃，对齐全站按钮语言）。
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
  children,
}: ResultViewProps) {
  return (
    <div className="flex select-none h-full w-full flex-col items-center justify-center" style={style}>
      <Result
        status={status}
        icon={<img src={STATUS_ICON[status]} alt="" className="mx-auto block h-[72px] w-[72px]" />}
        title={<span className="text-white">{title}</span>}
        subTitle={subtitle ? <span className="text-white">{subtitle}</span> : undefined}
        extra={[
          <MoleButton key="secondary" size="large" onClick={onSecondary} className="result-secondary-btn" style={{ width: 120, height: 40, borderRadius: 10 }}>
            {secondaryLabel}
          </MoleButton>,
          <MoleButton key="primary" type="primary" size="large" onClick={onPrimary} className="result-primary-btn" style={{ width: 120, height: 40, borderRadius: 10 }}>
            {primaryLabel}
          </MoleButton>,
        ]}
      />
      {/* 结果区下方附加内容（如失败明细）：与 Result 同列居中，负 margin 抵消 antd Result 底部留白 */}
      {children && <div className="-mt-8 w-full max-w-[460px] px-8">{children}</div>}
    </div>
  )
}