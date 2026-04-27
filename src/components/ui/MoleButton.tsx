import { Button, type ButtonProps } from 'antd'

interface MoleButtonProps extends ButtonProps {}

export function MoleButton({ className = '', ...rest }: MoleButtonProps) {
  return <Button className={className} {...rest} />
}

export type { ButtonProps as MoleButtonProps }
