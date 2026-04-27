import { ConfigProvider } from 'antd'
import { MoleButton } from '@/components/ui'

interface CleanFooterProps {
  onResetToDefault: () => void
}

/**
 * 底部操作区：分隔线 + 恢复至默认勾选态。
 * 由调用方控制渲染时机（仅 review 阶段）。
 */
export default function CleanFooter({ onResetToDefault }: CleanFooterProps) {
  return (
    <div className="pb-[12px] shrink-0">
      <div className="h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-10 w-full" />
      <div className="flex justify-end pt-3 pb-6">
        <ConfigProvider theme={{
          components: {
            Button: {
              defaultColor: '#dce1e8',           // 柔雾灰（首推）
              defaultHoverColor: '#ffffff',      // 悬停纯白
              defaultActiveColor: '#b0bcca',     // 点击冷灰（与常态形成明显色差）
            }
          }
        }}>
          <MoleButton
            size="small"
            color='default'
            variant='link'
            style={{ fontSize: 12 }}
            onClick={onResetToDefault}>
            恢复至默认勾选态
          </MoleButton>
        </ConfigProvider>
      </div>
    </div>
  )
}
