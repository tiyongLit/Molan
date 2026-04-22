import { MoleButton } from '@/components/ui'
import { useI18n } from '@/i18n'
import { Progress } from 'antd'
import type { GetProp, ProgressProps } from 'antd'
import type { ScanProgress } from '../../hooks/useAnalyzeData'

/**
 * 扫描进度浮层 — 标题 / 进度条 / 状态 / 取消 四行。
 *
 * 统一视觉（开始分析 / 重新扫描 / 钻取扫描共用）：
 *   1. 标题行：「正在扫描目录...」+ 右百分比（未知阶段显示 0%）
 *   2. 进度条：
 *      - 不确定态（首个进度事件到达前）：wobble 动画条（渐变填充左右滑动）
 *      - 确定态：antd Progress + 渐变填充（hue 随百分比从 200→0 青蓝→深蓝）
 *      两种状态共享相同高度(6px)、圆角(4px)、轨道色(rgba(255,255,255,0.3))
 *   3. 状态行：实时扫描路径（截断，title 提示全路径）
 *   4. 取消按钮：仅在传入 onCancel 时渲染（取消中禁用并显示「正在取消...」）
 */
interface ScanOverlayProps {
  progress: ScanProgress | null
  /** 取消中：已发取消请求、等后端 walker 退出（不假装已停） */
  cancelling?: boolean
  /** 取消回调；缺省时不渲染取消按钮 */
  onCancel?: () => void
}

const CANCEL_BTN_CLASS =
  '!h-6 !px-3 !rounded-[3px] !border !border-white/[0.12] !bg-white/[0.08] hover:!bg-white/[0.12] !text-white/70 hover:!text-white/85 shrink-0'

/** 渐变进度条样式 — 同 Clean ProgressLine：hue 随百分比从 200（青蓝）→ 0（深蓝） */
const progressStyles: ProgressProps['styles'] = (info): GetProp<ProgressProps, 'styles', 'Return'> => {
  const pct = info?.props?.percent ?? 0
  const _hue = 200 - (200 * pct) / 100
  void _hue
  return {
    root: { marginBlock: 0, paddingInline: 0 },
    track: {
      // backgroundImage: `linear-gradient(to right, hsla(${hue}, 85%, 65%, 1), hsla(${hue + 30}, 90%, 55%, 0.95))`,
      backgroundColor: '#ffffff',
      borderRadius: 4,
      transition: 'all 0.3s ease',
    },
    rail: {
      backgroundColor: 'rgba(255, 255, 255, 0.3)',
      borderRadius: 4,
    },
  }
}

// ── 不确定态 wobble 动画 keyframes（组件内注入，与 MoleProgress 同模式） ──
// 完全复刻 README Uiverse loader .line::after：
//   - 纯白实色（非渐变），borderRadius 圆角
//   - translateX(-90%) → translateX(90%) → translateX(-90%)
//   - 2s cubic-bezier(0.5, 0.8, 0.5, 0.2) infinite
//   - 两端快速进出、中段缓慢巡航
const WOBBLE_KF = `
@keyframes scan-overlay-wobble {
  0% { transform: translateX(-90%); }
  50% { transform: translateX(90%); }
  100% { transform: translateX(-90%); }
}
`
let kfInjected = false
function ensureKeyframes() {
  if (kfInjected || typeof document === 'undefined') return
  const el = document.createElement('style')
  el.textContent = WOBBLE_KF
  document.head.appendChild(el)
  kfInjected = true
}

/**
 * 不确定态进度条 — 固定圆角端盖 + 滑动白条：
 *   - 外层容器提供轨道色 + borderRadius:4（固定圆角）
 *   - 内层白条 #ffffff 以 translateX(±90%) 滑动
 *   - 左右各一个端盖 div（width=borderRadius=4px，与轨道同色），
 *     固定在两端，为滑动白条提供始终可见的圆角端头
 *   原理：overflow:hidden 裁切边界是直的，无法随 translateX 保留圆角，
 *     因此在父容器边缘放置固定圆角端盖，白条从端盖下方穿过，
 *     端盖与轨道同色时融入背景，白条经过时被端盖覆出圆角
 */
function IndeterminateBar() {
  ensureKeyframes()
  return (
    <div
      style={{
        position: 'relative',
        height: 6,
        width: '100%',
        overflow: 'hidden',
        borderRadius: 4,
        backgroundColor: 'rgba(255, 255, 255, 0.3)',
      }}
    >
      {/* 滑动白条 */}
      <div
        style={{
          height: '100%',
          width: '100%',
          backgroundColor: '#ffffff',
          animation: 'scan-overlay-wobble 2s cubic-bezier(0.5, 0.8, 0.5, 0.2) infinite',
        }}
      />

    </div>
  )
}

export function ScanOverlay({ progress, cancelling = false, onCancel }: ScanOverlayProps) {
  const { t } = useI18n()

  const cancelButton = onCancel ? (
    <MoleButton
      type="text"
      size="small"
      disabled={cancelling}
      onClick={onCancel}
      className={CANCEL_BTN_CLASS}
      style={{ fontSize: 11 }}
    >
      {cancelling ? t('analyze.scan.cancelling') : t('analyze.cancel')}
    </MoleButton>
  ) : null

  // 有效百分比：收到进度事件且 percent >= 0
  const hasValidPercent = !!progress && progress.percent >= 0
  const percent = hasValidPercent ? Math.min(progress!.percent, 99) : 0

  return (
    <div className="flex flex-col items-center gap-2 w-full max-w-[320px]">
      {/* ① 标题行：左标题 + 右百分比 */}
      <div className="flex items-center justify-between w-full text-sm">
        <span className="text-white/85">{t('analyze.scan.scanning')}</span>
        <span className="font-mono tabular-nums text-white/55">{percent}%</span>
      </div>

      {/* ② 进度条：不确定态 wobble / 确定态渐变 */}
      {hasValidPercent ? (
        <Progress size={{ height: 6 }} styles={progressStyles} percent={percent} showInfo={false} />
      ) : (
        <IndeterminateBar />
      )}

      {/* ③ 状态行：实时扫描路径 */}
      <p className="w-full text-xs text-white/45 truncate" title={progress?.current_path}>
        {progress?.current_path || ''}
      </p>

      {/* ④ 取消 */}
      {cancelButton}
    </div>
  )
}
