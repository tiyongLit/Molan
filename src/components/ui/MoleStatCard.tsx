import { Card, Statistic } from 'antd'

interface MoleStatCardProps {
  icon?: React.ReactNode
  title: string
  value: string | number
  color?: string
  className?: string
}

export function MoleStatCard({ icon, title, value, color, className = '' }: MoleStatCardProps) {
  return (
    <Card className={className}>
      <Statistic
        title={title}
        value={value}
        valueStyle={color ? { color } : undefined}
        prefix={icon}
      />
    </Card>
  )
}
