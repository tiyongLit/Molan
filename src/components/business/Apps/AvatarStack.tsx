import { Tooltip } from 'antd'
import { AppIcon } from './AppIcon'
import type { MoleListAppsEntry } from '@/types/mole'

export interface AvatarStackProps {
  /** 应用列表 */
  apps: MoleListAppsEntry[]
  /** 最多显示数量，默认 3 */
  maxDisplay?: number
  /** 图标尺寸，默认 24 */
  iconSize?: number
}

/**
 * 重叠头像堆栈：参考 GitHub avatar stack 模式
 * 
 * 功能：
 * - 默认状态：每个图标重叠前一个图标的 50%（向右重叠）
 * - Hover 状态：图标向右展开，间距更小（更丝滑）
 * - 徽标显示：红色通知徽标，显示在最后一个图标的右上角
 * - 平滑动画：CSS transition 实现 300ms 平滑过渡
 * - Tooltip：hover 时显示应用名称
 * - z-index 层叠：确保正确的层叠顺序
 */
export function AvatarStack({ apps, maxDisplay = 3, iconSize = 24 }: AvatarStackProps) {
  const displayApps = apps.slice(0, maxDisplay)
  const remainingCount = apps.length - maxDisplay
  const hasMore = remainingCount > 0

  return (
    <div className="group flex items-center">
      {displayApps.map((app, index) => {
        const isLast = index === displayApps.length - 1
        
        return (
          <Tooltip key={app.path} title={app.display_name || app.name}>
            <div 
              className={`
                relative rounded-md shrink-0
                transition-all duration-300 ease-in-out hover:scale-110
                ${index === 0 ? '' : `-ml-3 group-hover:-ml-1.5`}
                z-${(index + 1) * 10}
              `}
              style={{
                marginLeft: index === 0 ? 0 : undefined,
              }}
            >
              <AppIcon 
                name={app.display_name || app.name} 
                path={app.path} 
                size={iconSize} 
              />
              
              {/* 红色徽标：显示在最后一个图标的右上角 */}
              {isLast && hasMore && (
                <div className="absolute -top-1 -right-1 z-50">
                  <div 
                    className="flex items-center justify-center rounded-full bg-red-500 text-white text-[9px] font-bold shadow-lg"
                    style={{
                      width: iconSize * 0.6,
                      height: iconSize * 0.6,
                      minWidth: iconSize * 0.6,
                      minHeight: iconSize * 0.6,
                    }}
                  >
                    +{remainingCount}
                  </div>
                </div>
              )}
            </div>
          </Tooltip>
        )
      })}
    </div>
  )
}
