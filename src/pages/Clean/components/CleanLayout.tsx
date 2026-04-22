import type { ReactNode } from 'react'
import SimpleBar from 'simplebar-react'
import 'simplebar-react/dist/simplebar.min.css'
import { MoleButton, NineGridLoading } from '@/components/ui'
import { PAGE_THEME_VARS } from '@/constants/theme'
import { PRIMARY_CTA_STYLE } from '../clean.constants'
import ProgressLine from './ProgressLine'

/**
 * 头部主操作按钮配置。
 *
 * 由页面容器按 phase 生成（见 index.tsx 的视图配置段），
 * 布局层只负责渲染，不感知按钮语义。
 */
export interface CleanActionConfig {
  /** 按钮文案（如「立即扫描」「取消清理」） */
  label: string
  /** 点击回调 */
  onClick: () => void
  /** 是否主强调按钮（primary 样式）；默认否 */
  primary?: boolean
  /** 是否禁用 */
  disabled?: boolean
}

export interface CleanLayoutProps {
  /** 左上角九宫格装饰是否播放 loading 动画（scanning / cleaning 阶段为 true） */
  loading?: boolean
  /** 头部左侧状态文案插槽（通常为 CleanStatusBar，按 phase 切换内容） */
  statusContent: ReactNode
  /** 头部右侧主操作按钮配置 */
  action: CleanActionConfig
  /** 顶部进度条百分比（0-100）；为 0 且未强制显示时渲染 hairline 分隔线 */
  progress: number
  /** 强制渲染进度条（percent 为 0 也显示，如清理刚启动） */
  progressAlwaysShow?: boolean
  /** 滚动区内容（分类列表 / 扫描骨架列表） */
  children: ReactNode
  /** 底部固定区（滚动区之外；如 review 阶段的「恢复默认勾选」） */
  footer?: ReactNode
}

/**
 * 清理页统一布局壳：头部行 + 进度条 + 滚动列表区 + 底部插槽。
 *
 * 职责边界（关注点分离）：
 * - 本组件只负责布局骨架与插槽，**不感知任何 phase / 业务状态**；
 * - 状态文案（statusContent）、按钮（action）、列表内容（children）
 *   均由容器（index.tsx）按状态机生成后注入；
 * - idle / scanning / review / cleaning 四个阶段共用本壳，
 *   done 阶段因视觉结构完全不同走独立 ScanResult 视图。
 *
 * 滚动条贴边策略（方案 A，详见 styles/scrollbar.css）：
 * - 根容器不带水平 margin → SimpleBar 右缘贴窗口右缘，滚动条 overlay 不被裁剪；
 * - 固定区块（头部/进度条/footer）自带留白层（左 24 / 右 48，与旧根容器 margin 视觉一致）；
 * - 内容缩进由滚动区内层 wrapper 补偿（pl-24 / pr-48）。
 */
export default function CleanLayout({
  loading = false,
  statusContent,
  action,
  progress,
  progressAlwaysShow = false,
  children,
  footer,
}: CleanLayoutProps) {
  return (
    <div className="flex flex-col h-full overflow-hidden" style={PAGE_THEME_VARS}>
      {/* 固定头部区：头部行 + 进度条（自带留白，不参与滚动） */}
      <div className="shrink-0 ml-[24px] mr-[48px]">
        {/* 头部：loading 装饰 + 状态文案 | 主操作按钮 */}
        <div className="flex items-start justify-between pt-[32px] pb-[12px]">
          <div className="flex items-start gap-4">
            <div className="shrink-0">
              <NineGridLoading loading={loading} />
            </div>
            {statusContent}
          </div>

          <div className="flex items-center gap-3">
            <MoleButton
              type={action.primary ? 'primary' : undefined}
              size="large"
              className="clean-primary-btn"
              style={PRIMARY_CTA_STYLE}
              disabled={action.disabled}
              onClick={action.onClick}
            >
              {action.label}
            </MoleButton>
          </div>
        </div>

        <ProgressLine percent={progress} alwaysShow={progressAlwaysShow} />
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
