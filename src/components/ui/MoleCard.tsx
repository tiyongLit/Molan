import { Card, type CardProps } from 'antd'

interface MoleCardProps extends CardProps {}

export function MoleCard({ className = '', ...rest }: MoleCardProps) {
  return <Card className={className} {...rest} />
}

export type { CardProps as MoleCardProps }
