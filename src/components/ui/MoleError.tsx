import { Alert, type AlertProps } from 'antd'

interface MoleErrorProps {
  message?: string
  error?: string
  className?: string
  type?: AlertProps['type']
}

export function MoleError({
  message = '出错了',
  error,
  className = '',
  type = 'error'
}: MoleErrorProps) {
  return <Alert type={type} message={message} description={error} showIcon className={className} />
}
