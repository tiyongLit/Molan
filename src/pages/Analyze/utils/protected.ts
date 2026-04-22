/**
 * 判断一个条目是否受保护——直接读取 Rust 扫描时下发的 protected 字段。
 * 前端不做任何路径计算，避免误伤（如 ~/work/mail 的 name 是 "mail" 但不在系统 Library 下）。
 *
 * 调用方（EntryRow / toggleCheck / selectAll / deleteSelected / useContextMenu / useKeyboard）
 * 传入的 entry 必须有 protected 字段（所有 MoleAnalyzeEntry 都满足）。
 */
export function isProtectedEntrySync(entry: { protected?: boolean }): boolean {
  return entry.protected === true
}
