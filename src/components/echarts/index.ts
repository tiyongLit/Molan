/**
 * ECharts 封装组件统一出口。
 *
 * 所有 Dashboard（及未来模块）使用的 ECharts 图表组件
 * 统一从此目录导出，便于按需加载与 tree-shaking 管理。
 */

export { SparklineChart, type SparklineChartProps } from './SparklineChart'
