import { Tag, type TagProps } from 'antd'

interface MoleTagProps extends TagProps {}

export function MoleTag({ className = '', ...rest }: MoleTagProps) {
  return <Tag className={className} {...rest} />
}

export type { TagProps as MoleTagProps }
