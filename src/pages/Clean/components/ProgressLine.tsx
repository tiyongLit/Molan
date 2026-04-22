import { Progress, Flex } from 'antd'
import type { GetProp, ProgressProps } from 'antd'

// 进度条渐变样式：随百分比从青蓝 (hue 200) 过渡到深蓝 (hue 0)
interface ProgressLineProps {
  /** 进度百分比（0-100） */
  percent: number
  /** 强制渲染进度条（即使 percent 为 0，如清理刚开始时） */
  alwaysShow?: boolean
}

const progressStyles: ProgressProps['styles'] = (info): GetProp<ProgressProps, 'styles', 'Return'> => {
  const percent = info?.props?.percent ?? 0
  const hue = 200 - (200 * percent) / 100
  return {
    root: {
      marginBlock: 0,
      paddingInline: 0,
    },
    track: {
      backgroundImage: `
        linear-gradient(
          to right,
          hsla(${hue}, 85%, 65%, 1),
          hsla(${hue + 30}, 90%, 55%, 0.95)
        )`,
      borderRadius: 4,
      transition: 'all 0.3s ease',
    },
    rail: {
      backgroundColor: 'rgba(255, 255, 255, 0.3)',
      borderRadius: 4,
    },
  }
}

/**
 * 3px 进度条：渲染渐变进度，或在非进行中时渲染 hairline 分隔线。
 * 扫描/清理阶段共用。
 */
export default function ProgressLine({ percent, alwaysShow = false }: ProgressLineProps) {
  return (
    <Flex className="h-[3px]" align="flex-end">
      {alwaysShow || percent > 0 ? (
        <Progress size={{ height: 3 }} styles={progressStyles} percent={percent} showInfo={false} />
      ) : (
        <div className="h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-10 w-full" />
      )}
    </Flex>
  )
}
