/**
 * IconService —— 统一系统图标服务（单例）
 *
 * 缓存架构：
 *   1. emoji 兜底（staticIconMap）         — 永远可用，0ms
 *   2. 前端 LRU 内存缓存（< 200 条）       — 同 session 命中 0 IPC
 *   3. Rust AppCache（mtime 校验）         — 跨 session + 实时新鲜度
 *
 * 核心策略：
 *   - Rust mole_get_icons_batch: 单次 IPC 获取批量图标，内部 mtime 校验
 *   - LRU 200：防止内存无限膨胀
 *   - preloadIconsIdle: 分批 batch IPC（每批 1 次），批间 setTimeout(0) yield
 *   - 请求去重：并发请求同一路径只发一次 IPC
 */

import { invoke } from '@tauri-apps/api/core'

// ============================================================================
// 类型
// ============================================================================

export type IconSrc = string | null

const MAX_CACHE = 200
const BATCH_IDLE = 20 // 每次 IPC 最多取 20 个路径，批间 setTimeout(0) yield

// ============================================================================
// LRU 节点
// ============================================================================

class CacheEntry {
  base64: string
  prev: CacheEntry | null = null
  next: CacheEntry | null = null

  constructor(
    public path: string,
    base64: string
  ) {
    this.base64 = base64
  }
}

// ============================================================================
// 单例
// ============================================================================

class IconService {
  /** 内存缓存：path → CacheEntry */
  private cache = new Map<string, CacheEntry>()

  /** LRU 头尾 */
  private head: CacheEntry | null = null
  private tail: CacheEntry | null = null

  /** 请求去重：path → Promise */
  private pending = new Map<string, Promise<string | null>>()

  // ── LRU 操作 ──

  private touch(entry: CacheEntry): void {
    if (entry === this.head) return
    // 从旧位置摘下
    if (entry.prev) entry.prev.next = entry.next
    if (entry.next) entry.next.prev = entry.prev
    if (entry === this.tail) this.tail = entry.prev
    // 插到头部
    entry.prev = null
    entry.next = this.head
    if (this.head) this.head.prev = entry
    this.head = entry
    if (!this.tail) this.tail = entry
  }

  private evict(): void {
    if (!this.tail) return
    const old = this.tail
    this.cache.delete(old.path)
    if (old.prev) old.prev.next = null
    this.tail = old.prev
    if (!this.tail) this.head = null
  }

  private cacheSet(path: string, base64: string): void {
    const existing = this.cache.get(path)
    if (existing) {
      existing.base64 = base64
      this.touch(existing)
      return
    }

    if (this.cache.size >= MAX_CACHE) this.evict()

    const entry = new CacheEntry(path, base64)
    this.cache.set(path, entry)
    entry.next = this.head
    if (this.head) this.head.prev = entry
    this.head = entry
    if (!this.tail) this.tail = entry
  }

  private cacheGet(path: string): string | null {
    const entry = this.cache.get(path)
    if (!entry) return null
    this.touch(entry)
    return entry.base64
  }

  // ── 公共方法 ──

  /** 获取单个路径图标 */
  async getIcon(path: string): Promise<IconSrc> {
    if (!path) return null

    const mem = this.cacheGet(path)
    if (mem) return `data:image/png;base64,${mem}`

    const inflight = this.pending.get(path)
    if (inflight) return inflight

    const promise = this.loadSingle(path)
    this.pending.set(path, promise)
    try {
      return await promise
    } finally {
      this.pending.delete(path)
    }
  }

  /** 批量预加载（全并发，用于 App 启动固定路径） */
  async preloadIcons(paths: string[]): Promise<void> {
    if (paths.length === 0) return
    const map = await this.loadBatchRaw(paths)
    for (const [p, b64] of Object.entries(map)) {
      if (b64) this.cacheSet(p, b64)
    }
  }

  /** 批量预加载 — 分批 yield 版本，每批 BATCH_IDLE 个路径一次 IPC，批间 setTimeout(0) 让出事件循环 */
  async preloadIconsIdle(paths: string[]): Promise<void> {
    if (paths.length === 0) return
    for (let i = 0; i < paths.length; i += BATCH_IDLE) {
      const batch = paths.slice(i, i + BATCH_IDLE)
      const map = await this.loadBatchRaw(batch)
      for (const [p, b64] of Object.entries(map)) {
        if (b64) this.cacheSet(p, b64)
      }
      if (i + BATCH_IDLE < paths.length) {
        await new Promise<void>((resolve) => setTimeout(resolve, 0))
      }
    }
  }

  /** 同步读取内存缓存（useIconMap 专用） */
  getCachedSync(path: string): IconSrc {
    const b64 = this.cache.get(path)?.base64
    return b64 ? `data:image/png;base64,${b64}` : null
  }

  /** 清空内存缓存 */
  clearCache(): void {
    this.cache.clear()
    this.pending.clear()
    this.head = null
    this.tail = null
  }

  // ── 内部 ──

  private async loadSingle(path: string): Promise<IconSrc> {
    try {
      const batchResult = await invoke<Record<string, string | null>>('mole_get_icons_batch', {
        paths: [path]
      })
      const b64 = batchResult[path] ?? null
      if (b64) this.cacheSet(path, b64)
      return b64 ? `data:image/png;base64,${b64}` : null
    } catch {
      return null
    }
  }

  private async loadBatchRaw(paths: string[]): Promise<Record<string, string | null>> {
    try {
      return await invoke<Record<string, string | null>>('mole_get_icons_batch', { paths })
    } catch {
      const fallback: Record<string, string | null> = {}
      for (const p of paths) fallback[p] = null
      return fallback
    }
  }
}

export const iconService = new IconService()
