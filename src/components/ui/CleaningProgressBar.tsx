import { Progress } from 'antd'

interface CleaningProgressBarProps {
  /** 当前进度百分比（0-100） */
  percent: number
  /** 当前操作描述 */
  currentAction: string
}

/**
 * 内嵌进度条组件：用于在 app 行内显示清理进度。
 * 样式与 Uninstall 页面深色主题一致（emerald 渐变）。
 */
export function CleaningProgressBar({ percent, currentAction }: CleaningProgressBarProps) {
  return (
    <div className="w-full py-1 space-y-0.5">
      <div className="text-[10px] text-white/60">{currentAction}</div>
      <div className="flex items-center gap-2">
        <Progress
          percent={percent}
          showInfo={false}
          strokeColor={{
            '0%': '#64dfa7',
            '100%': '#00d899',
          }}
          railColor="rgba(255, 255, 255, 0.08)"
          size={{ height: 3 }}
          className="flex-1"
        />
        <span className="text-[10px] font-mono text-emerald-400 shrink-0">{percent}%</span>
      </div>
    </div>
  )
}
