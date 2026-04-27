import classnames from 'classnames'
import { formatSize } from '@/utils/format'
import { SEMANTIC_COLORS } from '@/constants/theme'
import { MoleButton } from '@/components/ui'
import './CleanStatusBar.css'

/**
 * 清理页头部左侧状态条：五个阶段共用一个组件，按 status 切换文案。
 *
 * 状态：
 * - idle      初始态（标题 + 副标题 + 错误提示占位）
 * - scanning  正在扫描（脉冲点 + 循环省略号 + 当前扫描目标 + 实时累计大小）
 * - cleaning  正在清理（当前清理项）
 * - ready     扫描完成、未勾选（共发现可清理文件，提示勾选）
 * - selected  扫描完成、已勾选（已选中大小 + 共可清理大小）
 *
 * 注：颜色与文案集中在 STATUS_CONFIG，便于后续按设计图对齐主题色。
 */

export type CleanStatus = 'idle' | 'scanning' | 'cleaning' | 'ready' | 'selected'

export interface CleanStatusBarProps {
  /** 当前展示状态 */
  status: CleanStatus
  /** idle 状态：扫描失败的错误信息（固定占位，避免列表抖动） */
  error?: string
  /** ready / selected 状态：可清理总大小 */
  totalSize?: number
  /** selected 状态：已选中大小 */
  selectedSize?: number
  /** scanning 状态：当前正在扫描的目标文案，如 "系统缓存" */
  scanTarget?: string
  /** scanning 状态：已累计扫描到的垃圾大小（KB），实时累加 */
  accumulatedSizeKb?: number
  /** cleaning 状态：当前正在清理的项 */
  cleanCurrent?: string
  /** ready / selected 状态：返回按钮点击回调 */
  onBack?: () => void
  className?: string
}

interface StatusConfig {
  /** 主标题前缀文案 */
  title: string
  /** 副标题文案（仅 idle 状态使用） */
  subtitle?: string
  /** 数值高亮色 */
  accent: string
  /** 标题色 */
  titleColor: string
}

const STATUS_CONFIG: Record<CleanStatus, StatusConfig> = {
  idle: {
    title: '清理各种系统垃圾',
    subtitle: '请选择需要扫描的垃圾分类',
    accent: '#8CB4DC',
    titleColor: '#FFFFFF',
  },
  scanning: {
    title: '已发现',
    accent: '#8CB4DC',
    titleColor: '#FFFFFF',
  },
  cleaning: {
    title: '正在清理中，请稍后',
    accent: '#8CB4DC',
    titleColor: '#FFFFFF',
  },
  ready: {
    title: '共发现可清理文件',
    accent: '#FFAA00',
    titleColor: '#FFFFFF',
  },
  selected: {
    title: '共发现可清理文件',
    accent: '#FFAA00',
    titleColor: '#FFFFFF',
  },
}

export default function CleanStatusBar({
  status,
  error,
  totalSize = 0,
  selectedSize = 0,
  scanTarget,
  accumulatedSizeKb = 0,
  cleanCurrent,
  onBack,
  className,
}: CleanStatusBarProps) {
  const config = STATUS_CONFIG[status]

  return (
    <div className={classnames('pt-1', className)}>
      {status === 'idle' ? (
        <div className="flex flex-col min-h-[80px]">
          <h1 className="text-2xl font-semibold leading-tight" style={{ color: config.titleColor }}>
            {config.title}
          </h1>
          <p className="mt-2 text-sm text-white/60">{config.subtitle}</p>
          {/* 错误提示固定占位（min-h-[16px]），避免出现/消失时列表下移 */}
          <div className="mt-2 min-h-[16px] flex items-center">
            {error ? (
              <p className="text-xs font-medium" style={{ color: SEMANTIC_COLORS.dangerRed }}>{error}</p>
            ) : null}
          </div>
        </div>
      ) : status === 'scanning' ? (
        <>
          <h1 className="text-2xl font-semibold leading-tight" style={{ color: config.titleColor }}>
            {config.title} {formatSize(accumulatedSizeKb * 1024)} 可清理垃圾
          </h1>
          <div className="mt-2 flex items-center gap-2">
            <span className="text-sm text-white/60">
              正在扫描{scanTarget ? ` ${scanTarget}` : ''}
              <span className="clean-status__ellipsis" aria-hidden>
                <span>.</span>
                <span>.</span>
                <span>.</span>
              </span>
            </span>
          </div>
        </>
      ) : status === 'cleaning' ? (
        <>
          <h1 className="text-2xl font-semibold leading-tight" style={{ color: config.titleColor }}>
            {config.title}<span className="clean-status__ellipsis" aria-hidden><span>.</span><span>.</span><span>.</span></span>
          </h1>
          <div className="mt-2 flex items-center gap-2">
            <span className="text-sm text-white/60">
              正在清理{cleanCurrent ? ` ${cleanCurrent}` : ''}
            </span>
          </div>
        </>
      ) : (
        <>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-semibold leading-tight" style={{ color: config.titleColor }}>
              {config.title} {formatSize(totalSize)}
            </h1>
            {onBack && (
              <MoleButton
                size="small"
                variant="outlined"
                className="clean-back-btn"
                onClick={onBack}
              >
                返回
              </MoleButton>
            )}
          </div>
          {status === 'selected' ? (
            <div className="mt-2 flex items-center gap-1">
              <span className="text-sm text-white/60">已选中</span>
              <span className="text-sm font-semibold" style={{ color: config.accent }}>
                {formatSize(selectedSize)}
              </span>
            </div>
          ) : (
            <div className="mt-2 flex items-center gap-1">
              <span className="text-sm text-white/60">请勾选左侧项目开始清理</span>
            </div>
          )}
        </>
      )}
    </div>
  )
}
