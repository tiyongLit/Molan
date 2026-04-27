import { Empty, type EmptyProps } from 'antd'

interface MoleEmptyProps extends EmptyProps {
  action?: React.ReactNode
}

export function MoleEmpty({ action, ...rest }: MoleEmptyProps) {
  return <Empty {...rest}>{action ? <div style={{ marginTop: 16 }}>{action}</div> : null}</Empty>
}

export type { EmptyProps as MoleEmptyProps }
