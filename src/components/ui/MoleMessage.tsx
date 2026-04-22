import { message } from 'antd'
import type { MessageInstance } from 'antd/es/message/interface'

/**
 * 模块级 message 实例 — 继承 ConfigProvider 主题
 *
 * 用法：
 *   import { moleMessage } from '@/components/ui'
 *   moleMessage.success('xxx')
 *   moleMessage.error('xxx')
 */

let msg: MessageInstance | null = null

/** 在 App.tsx 的 AntdApp 内部挂载，提供主题化的 context holder */
export function MoleMessageProvider({ children }: { children: React.ReactNode }) {
  const [api, contextHolder] = message.useMessage()
  msg = api
  return (
    <>
      {contextHolder}
      {children}
    </>
  )
}

/** 全局 message 代理 — 可在任意组件/hook/工具函数中调用 */
export const moleMessage = new Proxy({} as MessageInstance, {
  get(_, prop) {
    return (...args: any[]) => {
      if (msg && typeof msg[prop as keyof MessageInstance] === 'function') {
        ;(msg[prop as keyof MessageInstance] as any)(...args)
      }
    }
  }
})
