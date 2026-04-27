/**
 * ECharts 迷你趋势图封装（tree-shaken 按需加载）。
 *
 * 仅引入 LineChart + GridComponent + CanvasRenderer（≈50 KB gzip），
 * 关闭所有非必要组件（xAxis/yAxis/tooltip/legend/axisPointer）以获得
 * 最轻量的 sparkline 渲染路径。
 *
 * 支持：area / bars 双风格、mirror 镜像翻转、domainMax 固定纵轴、
 * 平滑动画（数据推入时自动过渡）。
 *
 * 对齐 Lemon Cleaner NetworkPlotView 的 Canvas 绘制语义：
 * - 上传：正常朝向，高峰朝上
 * - 下载：mirror=true，Y 轴反转，高峰朝下（upsideDown=YES）
 */

import ReactEChartsCore from 'echarts-for-react/lib/core'
import * as echarts from 'echarts/core'
import { LineChart, BarChart } from 'echarts/charts'
import { GridComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import type { CSSProperties } from 'react'

// 按需注册：折线 + 柱状 + 直角坐标系 + Canvas 渲染器
echarts.use([LineChart, BarChart, GridComponent, CanvasRenderer])

export interface SparklineChartProps {
  /** 数据序列（数值数组，单位由调用方决定） */
  data: number[]
  /** 静态描边颜色（colorFn 未传时使用） */
  color: string
  /** 图表高度（px） */
  height?: number
  /** 图表风格：'area'（折线 + 面积填充）/ 'bars'（柱状） */
  style?: 'area' | 'bars'
  /** Y 轴固定上限（不传则自适应序列最大值） */
  domainMax?: number
  /** 镜像翻转（Lemon upsideDown=YES 同款）：低值在顶部、高值在底部 */
  mirror?: boolean
  /** 面积填充背景色（仅 area 风格有效）；不传则自动从描边色派生 ~13% 透明度 */
  backgroundColor?: string
  /** 动态颜色回调：根据最新数据值计算描边色（优先于 color） */
  colorFn?: (latestValue: number) => string
}

/** 空态占位：一条低透明度底线，保持图表区高度稳定 */
function Placeholder({ height, color }: { height: number; color: string }) {
  return (
    <div
      style={{
        height,
        borderBottom: `1px solid ${color}`,
        opacity: 0.25,
        marginTop: height - 1
      }}
    />
  )
}

export function SparklineChart({
  data,
  color,
  height = 16,
  style = 'area',
  domainMax,
  mirror,
  backgroundColor,
  colorFn
}: SparklineChartProps) {
  // 动态颜色：colorFn 优先，否则回退静态 color
  const effectiveColor = colorFn && data.length > 0
    ? colorFn(data[data.length - 1])
    : color

  if (data.length < 2) {
    return <Placeholder height={height} color={effectiveColor} />
  }

  const isArea = style === 'area'
  const maxVal = domainMax ?? Math.max(...data, 0.0001)

  const option: echarts.EChartsCoreOption = {
    grid: { top: 0, right: 0, bottom: 0, left: 0, containLabel: false },
    xAxis: {
      type: 'category',
      show: false,
      boundaryGap: !isArea, // 折线: false（紧贴两端）; 柱状: true（柱宽留空）
      data: data.map((_, i) => i)
    },
    yAxis: {
      type: 'value',
      show: false,
      min: 0,
      max: maxVal,
      inverse: !!mirror
    },
    series: [
      {
        type: isArea ? 'line' : 'bar',
        data,
        showSymbol: false,
        smooth: isArea ? 0.3 : false,
        // area 风格：描边 + 半透明面积填充（backgroundColor 优先，否则从描边色派生）
        lineStyle: isArea ? { width: 1.2, color: effectiveColor } : undefined,
        areaStyle: isArea
          ? { color: backgroundColor ?? `${effectiveColor}22` }
          : undefined,
        // bars 风格：圆角柱 + 低透明度填充
        itemStyle: !isArea
          ? { color: effectiveColor, borderRadius: [1, 1, 0, 0] }
          : undefined,
        barWidth: !isArea ? '60%' : undefined
      }
    ],
    // 关闭所有非必要组件，最小化渲染开销
    animation: true,
    animationDuration: 300,
    animationEasing: 'linear',
    animationThreshold: 100
  }

  const containerStyle: CSSProperties = {
    width: '100%',
    height,
    overflow: 'hidden'
  }

  return (
    <div style={containerStyle}>
      <ReactEChartsCore
        echarts={echarts}
        option={option}
        style={{ width: '100%', height: '100%' }}
        opts={{ renderer: 'canvas' }}
        notMerge={false}
        lazyUpdate
      />
    </div>
  )
}
