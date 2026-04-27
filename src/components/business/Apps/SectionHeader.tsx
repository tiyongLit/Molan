export interface SectionHeaderProps {
  /** 分区标题（英文，uppercase 展示） */
  title: string
  /** 分区条目数 */
  count: number
}

/**
 * 列表分区标题：大写标题 + 计数。
 *
 * 统一「更新 / 启动项」两个 tab 的分区标题结构（原两处实现
 * 存在 tracking / 颜色 / 字体等细微漂移），保证样式一致。
 */
export function SectionHeader({ title, count }: SectionHeaderProps) {
  return (
    <div className="flex items-center gap-1.5 px-[24px] pt-3.5 pb-1.5 select-none">
      <span className="text-[10px] font-bold tracking-wider text-white/60 uppercase">{title}</span>
      <span className="text-[10px] font-mono text-white/60">{count}</span>
    </div>
  )
}
