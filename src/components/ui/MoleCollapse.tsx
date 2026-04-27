import { Collapse, type CollapseProps } from 'antd'

interface MoleCollapseProps extends CollapseProps {}

export function MoleCollapse({ className = '', ...rest }: MoleCollapseProps) {
  return <Collapse className={className} {...rest} />
}

export type { CollapseProps as MoleCollapseProps }
