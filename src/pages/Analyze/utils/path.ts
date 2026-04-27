import type { MoleAnalyzeEntry } from '@/types/mole'
import type { BreadcrumbItem } from '../typings'

/** 将当前路径解析为面包屑，以 overview entry 为根 */
export function parseBreadcrumb(
  rootPath: string,
  currentPath: string,
  rootEntry?: MoleAnalyzeEntry
): BreadcrumbItem[] {
  if (!rootPath) return []

  // 首项：overview entry 或退化为路径最后一段
  const rootName = rootEntry?.name ?? rootPath.split('/').filter(Boolean).pop() ?? rootPath
  const items: BreadcrumbItem[] = [{ name: rootName, path: rootPath }]

  if (currentPath === rootPath) {
    return items
  }

  // 相对路径
  const rel = currentPath.startsWith(rootPath)
    ? currentPath.substring(rootPath.length)
    : currentPath

  const parts = rel.split('/').filter(Boolean)
  let acc = rootPath.endsWith('/') ? rootPath.slice(0, -1) : rootPath

  for (const part of parts) {
    acc += '/' + part
    items.push({ name: part, path: acc })
  }

  return items
}

/** 将大文件对象转为 MoleAnalyzeEntry 类型 */
export function largeFileToEntry(f: {
  name: string
  path: string
  size: number
}): MoleAnalyzeEntry {
  return {
    name: f.name,
    path: f.path,
    size: f.size,
    is_dir: false,
    insight: false,
    cleanable: false,
    protected: false
  }
}
