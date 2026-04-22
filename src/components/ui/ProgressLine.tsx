import { Progress, Flex } from 'antd'
import type { GetProp, ProgressProps } from 'antd'

export interface ProgressLineProps {
  /** 进度百分比（0-100） */
  percent: number
  /** 强制渲染进度条（即使 percent 为 0，如扫描/优化刚开始时） */
  alwaysShow?: boolean
  /** 0% 进度时的主色相（渐变左端），默认 200（Clean 青蓝） */
  hueStart?: number
  /** 100% 进度时的主色相（随进度线性过渡），默认 0（Clean 深蓝） */
  hueEnd?: number
  /** 渐变第二段色相对于主色相的偏移量，默认 30（Clean 蓝）；Optimize 橙传 20 */
  hueSpread?: number
  /** 进度条左右内边距（px），默认 0（边距由布局壳统一控制） */
  paddingInline?: number
}

// 饱和度/亮度统一取值（青蓝与橙色色相共用，视觉差异可忽略）
const SATURATION_START = 90
const LIGHTNESS_START = 60
const SATURATION_END = 90
const LIGHTNESS_END = 55

/**
 * 3px 进度条：渲染渐变进度，或在非进行中时渲染 hairline 分隔线。
 *
 * Clean（青蓝）与 Optimize（橙）的进度条结构完全一致，仅色相与内边距不同，
 * 故提取为通用组件：通过 `hueStart` / `hueEnd` / `hueSpread` 注入主题色相。
 *
 * @example
 * <ProgressLine percent={60} />                                                     // 青蓝
 * <ProgressLine percent={60} hueStart={30} hueEnd={15} hueSpread={20} />             // 橙
 */
export function ProgressLine({
  percent,
  alwaysShow = false,
  hueStart = 200,
  hueEnd = 0,
  hueSpread = 30,
  paddingInline = 0,
}: ProgressLineProps) {
  const progressStyles: ProgressProps['styles'] = (info): GetProp<ProgressProps, 'styles', 'Return'> => {
    const pct = info?.props?.percent ?? 0
    const hue = hueStart + ((hueEnd - hueStart) * pct) / 100
    return {
      root: {
        marginBlock: 0,
        paddingInline,
      },
      track: {
        backgroundImage: `
          linear-gradient(
            to right,
            hsla(${hue}, ${SATURATION_START}%, ${LIGHTNESS_START}%, 1),
            hsla(${hue + hueSpread}, ${SATURATION_END}%, ${LIGHTNESS_END}%, 0.95)
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