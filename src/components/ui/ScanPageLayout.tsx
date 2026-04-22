import type { CSSProperties, ReactNode } from 'react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { MoleButton, NineGridLoading } from '@/components/ui'
import { PAGE_THEME_VARS } from '@/constants/theme'
import { ProgressLine } from './ProgressLine'

/**
 * 头部主操作按钮配置。
 *
 * 由页面容器按 phase 生成，布局层只负责渲染，不感知按钮语义。
 */
export interface ScanPageActionConfig {
  /** 按钮文案（如「立即扫描」「开始优化（N）」） */
  label: string
  /** 点击回调 */
  onClick: () => void
  /** 是否主强调按钮（primary 样式）；默认否 */
  primary?: boolean
  /** 是否禁用 */
  disabled?: boolean
}

export interface ScanPageLayoutProps {
  /** 左上角九宫格装饰是否播放 loading 动画（scan / execute 阶段为 true） */
  loading?: boolean
  /** 九宫格装饰主色（Clean 默认白，Optimize 传 #fb923c） */
  loadingColor?: string
  /** 头部左侧状态文案插槽（按 phase 切换内容） */
  statusContent: ReactNode
  /** 头部右侧主操作按钮配置 */
  action: ScanPageActionConfig
  /** 主按钮 className（页面级品牌色样式，如 clean-primary-btn / optimize-primary-btn） */
  actionClassName?: string
  /** 主按钮内联样式（统一尺寸等） */
  actionStyle?: CSSProperties
  /** 顶部进度条百分比（0-100）；为 0 且未强制显示时渲染 hairline 分隔线 */
  progress: number
  /** 强制渲染进度条（percent 为 0 也显示，如执行刚启动） */
  progressAlwaysShow?: boolean
  /** 进度条渐变主色相（0% 进度端），Clean 200 / Optimize 30 */
  progressHueStart?: number
  /** 进度条渐变主色相（100% 进度端），Clean 0 / Optimize 15 */
  progressHueEnd?: number
  /** 进度条第二段色相偏移，Clean 30 / Optimize 20 */
  progressHueSpread?: number
  /** 性能诊断横条插槽（Optimize preview 阶段，无瓶颈时传 undefined） */
  banner?: ReactNode
  /** 滚动区内容（分类/分组列表 / 骨架列表） */
  children: ReactNode
  /** 底部固定区（滚动区之外；如「恢复默认」或日志面板） */
  footer?: ReactNode
}

/**
 * 扫描页统一布局壳：头部行 + 进度条 + 诊断横条插槽 + 滚动列表区 + 底部插槽。
 *
 * 职责边界（关注点分离）：
 * - 本组件只负责布局骨架与插槽，**不感知任何 phase / 业务状态**；
 * - 状态文案（statusContent）、按钮（action）、诊断横条（banner）、
 *   列表内容（children）、底部（footer）均由容器（页面 index.tsx）按状态机生成后注入；
 * - Clean 与 Optimize 的 idle / scanning / review / cleaning 阶段共用本壳，
 *   done 阶段因视觉结构完全不同走各自独立 Result 视图。
 *
 * 滚动条贴边策略（方案 A，详见 styles/scrollbar.css）：
 * - 根容器不带水平 margin → SimpleBar 右缘贴窗口右缘，滚动条 overlay 不被裁剪；
 * - 固定区块（头部/进度条/banner/footer）自带留白层（左 24 / 右 48，与旧根容器 margin 视觉一致）；
 * - 内容缩进由滚动区内层 wrapper 补偿（pl-24 / pr-48）。
 */
export function ScanPageLayout({
  loading = false,
  loadingColor = '#ffffff',
  statusContent,
  action,
  actionClassName,
  actionStyle,
  progress,
  progressAlwaysShow = false,
  progressHueStart = 200,
  progressHueEnd = 0,
  progressHueSpread = 30,
  banner,
  children,
  footer,
}: ScanPageLayoutProps) {
  return (
    <div className="flex flex-col h-full overflow-hidden" style={PAGE_THEME_VARS}>
      {/* 固定头部区：头部行 + 进度条 + 诊断横条（自带留白，不参与滚动） */}
      <div className="shrink-0 ml-[24px] mr-[48px]">
        {/* 头部：loading 装饰 + 状态文案 | 主操作按钮 */}
        <div className="flex items-start justify-between pt-[32px] pb-[12px]">
          <div className="flex items-start gap-4">
            <div className="shrink-0">
              <NineGridLoading loading={loading} color={loadingColor} />
            </div>
            {statusContent}
          </div>

          <div className="flex items-center gap-3">
            <MoleButton
              type={action.primary ? 'primary' : undefined}
              size="large"
              className={actionClassName}
              style={actionStyle}
              disabled={action.disabled}
              onClick={action.onClick}
            >
              {action.label}
            </MoleButton>
          </div>
        </div>

        <ProgressLine
          percent={progress}
          alwaysShow={progressAlwaysShow}
          hueStart={progressHueStart}
          hueEnd={progressHueEnd}
          hueSpread={progressHueSpread}
        />

        {/* 性能诊断瓶颈提示（Optimize preview） */}
        {banner}
      </div>

      {/* 列表滚动区：贴窗口边缘，滚动条 overlay（默认隐藏，滚动/悬停时显示） */}
      <SimpleBar className="mole-scroll flex-1 min-h-0" style={{ maxHeight: '100%' }}>
        <div className="pl-[24px] pr-[48px] pb-4">{children}</div>
      </SimpleBar>

      {footer && (
        <div className="shrink-0 ml-[24px] mr-[48px]">{footer}</div>
      )}
    </div>
  )
}
