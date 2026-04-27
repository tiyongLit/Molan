import { Drawer, type DrawerProps } from 'antd'

interface MoleDrawerProps extends DrawerProps {}

export function MoleDrawer({ className = '', ...rest }: MoleDrawerProps) {
  return <Drawer className={className} {...rest} />
}

export type { DrawerProps as MoleDrawerProps }
