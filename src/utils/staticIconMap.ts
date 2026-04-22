/**
 * 静态图标映射 —— 纯函数，不依赖 Tauri 后端。
 *
 * 设计目标：
 *   1. 使用 emoji 模拟 macOS 系统图标的语义
 *   2. 按路径模式匹配，覆盖常见系统目录和文件类型
 *   3. 函数签名与动态版本入口同构，方便未来替换为 Data URI
 */

const EXACT_MAP: Record<string, string> = {
  '/': '\u{1F5C4}',
  '/System': '\u{2699}',
  '/Applications': '\u{1F4BB}',
  '/Library': '\u{1F4DA}',
  '/Users': '\u{1F465}',
  '/opt': '\u{1F4E6}',
  '/usr': '\u{1F527}',
  '/private': '\u{1F512}'
}

const NAME_MAP: Record<string, string> = {
  Desktop: '\u{1F5A5}',
  Documents: '\u{1F4C4}',
  Downloads: '\u{2B07}',
  Movies: '\u{1F3AC}',
  Music: '\u{1F3B5}',
  Pictures: '\u{1F5BC}',
  Public: '\u{1F310}',
  Sites: '\u{1F310}',
  Library: '\u{1F4DA}',
  Applications: '\u{1F4BB}',
  '.Trash': '\u{1F5D1}',
  '.cargo': '\u{1F6E0}',
  '.rustup': '\u{1F980}',
  '.npm': '\u{1F4E6}',
  '.cache': '\u{1F9F9}',
  '.local': '\u{1F3E0}',
  '.config': '\u{2699}',
  '.ssh': '\u{1F510}',
  '.vscode': '\u{1F58A}',
  '.oh-my-zsh': '\u{1F41A}',
  '.docker': '\u{1F433}',
  '.git': '\u{1F4E1}',
  '.pnpm': '\u{1F4E6}',
  '.yarn': '\u{1F4E6}',
  '.bun': '\u{1F35E}',
  '.nvm': '\u{1F30F}',
  '.pyenv': '\u{1F40D}',
  '.go': '\u{1F438}',
  '.gem': '\u{1F48E}',
  '.android': '\u{1F4F1}',
  '.gradle': '\u{1F3D7}',
  '.m2': '\u{2615}',
  '.kube': '\u{2601}',
  '.aws': '\u{2601}',
  '.terraform': '\u{1F30D}',
  '.cursor': '\u{270F}',
  '.trae-cn': '\u{1F916}',
  '.lingma': '\u{1F9E0}',
  '.ai_completion': '\u{2728}',
  '.stash': '\u{1F4CC}',
  '.iterm2': '\u{1F4BB}',
  '.zsh_sessions': '\u{1F4AD}',
  '.ollama': '\u{1F916}',
  '.lmstudio': '\u{1F916}'
}

function isHidden(name: string): boolean {
  return name.startsWith('.')
}

function isAppBundle(name: string): boolean {
  return name.endsWith('.app')
}

function isXcodeProject(name: string): boolean {
  return name.endsWith('.xcodeproj') || name.endsWith('.xcworkspace')
}

export interface IconInput {
  path: string
  name: string
  isDir: boolean
}

export function resolveStaticIcon(input: IconInput): string {
  const { path, name, isDir } = input
  if (EXACT_MAP[path]) return EXACT_MAP[path]
  if (NAME_MAP[name]) return NAME_MAP[name]
  if (!isDir) {
    if (isAppBundle(name)) return '\u{1F4E6}'
    if (isXcodeProject(name)) return '\u{1F528}'
    return '\u{1F4C4}'
  }
  if (isHidden(name)) return '\u{1F4C2}'
  return '\u{1F4C1}'
}

export function resolveStaticIconMap(inputs: IconInput[]): Record<string, string> {
  const map: Record<string, string> = {}
  for (const input of inputs) {
    map[input.path] = resolveStaticIcon(input)
  }
  return map
}
