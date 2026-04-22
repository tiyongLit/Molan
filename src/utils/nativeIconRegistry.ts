/**
 * NativeIconRegistry —— 内容寻址原生图标前端注册表（单例）。
 *
 * # 数据模型（与 Rust 后端 `native_icon_registry.rs` 镜像）
 *
 *   pathIndex:     Map<path, contentID>     // 路径 → 内容指纹（128 位 hash hex）
 *   contentStore:  Map<contentID, string>   // 内容指纹 → SVG data URI
 *
 * 同一像素内容 = 同一 contentID，天然去重：
 *   - 普通文件夹图标 583 个路径在 pathIndex 里各占一条（轻量），
 *   - 在 contentStore 里只占一条（重），SVG data URI ~30KB × 108 种 ≈ 3MB。
 *
 * # 生命周期
 *
 * 应用启动即构造（懒加载）。首次 `resolve(paths)` 时通过
 * `mole_native_icons_resolve` IPC 拿回 entries + contents；合并到本地缓存；
 * 订阅者（React hook）收到通知后重渲染，`<img src={getSync(path)} />` 命中。
 *
 * 后续同一 path 再次 `resolve`：
 *   - pathIndex 已命中且内容在 store → 跳过；
 *   - pathIndex 命中但内容缺失（跨会话短窗口）→ 重新拉取（后端会回传内容）；
 *   - 后端 mtime 变化 → 返回新 contentID + 新 SVG；
 *   - 前端合并覆盖旧条目，订阅者再次触发重渲染。
 *
 * 后端契约：`resolve` 响应的 contents **覆盖本次 entries 引用的全部内容**
 * （不只是新编码项），因此一次前端会话里同一份内容最多传输一次，合并幂等。
 *
 * # 与旧 `iconService.ts` 的区别
 *
 *   |                | iconService（旧）        | nativeIconRegistry（新）         |
 *   |----------------|--------------------------|----------------------------------|
 *   | 缓存键          | path                     | contentID（内容寻址）            |
 *   | 数据格式        | PNG base64               | SVG data URI（PNG 包 SVG 信封）  |
 *   | 去重            | 无                       | 同内容只存一份                    |
 *   | 体积（583 路径）| 583 × 870KB = 490MB      | 1 × 30KB = 30KB                  |
 *   | IPC payload     | 20 路径 × 870KB = 17MB   | 典型 ~50KB（去重后）             |
 *
 * # Emoji 兜底
 *
 * 本注册表只存原生图标。Emoji 兜底交给调用方：`NativeIcon` 组件在
 * `getSync(path)` 返回 null 时调 `resolveStaticIcon(input)` 取 emoji。
 * （保持关注点分离：注册表专注原生图标，emoji 映射归 staticIconMap 模块）
 */

import { invoke } from '@tauri-apps/api/core'
import { CMD_MOLE_NATIVE_ICONS_RESOLVE } from '@/constants/tauri-commands'

// ============================================================================
// 类型
// ============================================================================

/** 与 Rust `native_icon_registry::NativeIconsResponse` 对齐。 */
interface NativeIconsResponse {
  entries: Record<string, string> // path → contentID
  contents: Record<string, string> // contentID → SVG data URI
}

/** SVG data URI 字符串（`data:image/svg+xml;base64,...`）或 null（未命中）。 */
export type IconSrc = string | null

// ============================================================================
// 单例
// ============================================================================

class NativeIconRegistry {
  /** 内容存储：contentID → SVG data URI */
  private contentStore = new Map<string, string>()

  /** 路径索引：path → contentID */
  private pathIndex = new Map<string, string>()

  /** 飞行中请求：同一批路径只发一次 IPC（去重 + 防抖） */
  private pending = new Map<string, Promise<string | null>>()

  /** 订阅者：内容变更时通知（React hook 据此重渲染） */
  private subscribers = new Set<() => void>()

  /** 内容版本号：任何写入（含 pathIndex 重指向）自增，作 useSyncExternalStore 快照缓存的失效键 */
  private _version = 0

  // ── 公共读取 ──

  /** 同步读取：path → SVG data URI 或 null。O(1) 字典查询。 */
  get(path: string): IconSrc {
    if (!path) return null
    const cid = this.pathIndex.get(path)
    if (!cid) return null
    return this.contentStore.get(cid) ?? null
  }

  /** 批量同步读取（useNativeIconMap 用） */
  getMany(paths: string[]): Record<string, IconSrc> {
    const out: Record<string, IconSrc> = {}
    for (const p of paths) out[p] = this.get(p)
    return out
  }

  /** 当前内容版本号（快照缓存的失效键） */
  get version(): number {
    return this._version
  }

  /** 路径是否已完整解析：pathIndex 命中且对应内容已在 contentStore */
  isResolved(path: string): boolean {
    const cid = this.pathIndex.get(path)
    return !!cid && this.contentStore.has(cid)
  }

  // ── 解析 ──

  /**
   * 批量解析：一次 IPC 拿全量结果，合并到本地缓存。
   *
   * @returns 是否有新内容写入（调用方据此决定是否触发重渲染）
   */
  async resolve(paths: string[]): Promise<boolean> {
    // 未完整解析（无索引 或 内容缺失）的路径都算 miss —— 后者是跨会话
    // 短窗口的补取场景，保证 emoji 兜底可恢复。
    const misses = paths.filter((p) => p && !this.isResolved(p))
    if (misses.length === 0) return false
    return this.fetchAndMerge(misses)
  }

  /**
   * 分批解析 + 批间 yield（不阻塞事件循环，对齐旧 `iconService.preloadIconsIdle`）。
   *
   * 每批 BATCH_SIZE 个路径一次 IPC，批间 `setTimeout(0)` 让出事件循环；
   * 典型 Analyze 目录 200 路径，~10 批 × ~50KB = 总 payload 500KB，
   * 单批 JSON.parse < 5ms，滚动/点击零干扰。
   *
   * **订阅者合并：整轮静默写入 + 末尾一次 notify**。
   * 原实现每批都 notify，100 路径 = 5 次订阅者风暴，
   * 上层 React 树被反复重渲染（iconMap → treemapItems → analyzeValue）。
   * 改为整轮只 notify 一次，重渲染从 N 批降为 1 次。
   *
   * @returns 是否有新内容写入
   */
  async resolveIdle(paths: string[]): Promise<boolean> {
    const BATCH_SIZE = 20
    const misses = paths.filter((p) => p && !this.isResolved(p))
    if (misses.length === 0) return false
    let anyAdded = false
    for (let i = 0; i < misses.length; i += BATCH_SIZE) {
      const batch = misses.slice(i, i + BATCH_SIZE)
      // silent=true：批写入不通知，避免多批风暴
      const added = await this.fetchAndMerge(batch, true)
      if (added) anyAdded = true
      if (i + BATCH_SIZE < misses.length) {
        await new Promise<void>((r) => setTimeout(r, 0))
      }
    }
    // 整轮结束后统一通知一次
    if (anyAdded) this.notify()
    return anyAdded
  }

  /** 单个解析（带请求去重：并发请求同一路径只发一次 IPC） */
  async resolveSingle(path: string): Promise<IconSrc> {
    if (!path) return null
    const cached = this.get(path)
    if (cached) return cached
    const inflight = this.pending.get(path)
    if (inflight) return inflight
    const promise = this.fetchAndMerge([path]).then(() => this.get(path))
    this.pending.set(path, promise)
    try {
      return await promise
    } finally {
      this.pending.delete(path)
    }
  }

  // ── 订阅（React hook 用） ──

  subscribe(cb: () => void): () => void {
    this.subscribers.add(cb)
    return () => {
      this.subscribers.delete(cb)
    }
  }

  // ── 调试/诊断 ──

  /** 统计：pathIndex 大小（路径数）、contentStore 大小（唯一图标数） */
  stats(): { paths: number; unique: number } {
    return { paths: this.pathIndex.size, unique: this.contentStore.size }
  }

  /** 清空所有缓存（测试 / 登出时调用） */
  clear(): void {
    this.contentStore.clear()
    this.pathIndex.clear()
    this.pending.clear()
    this.notify()
  }

  // ── 内部 ──

  /**
   * 发 IPC 拿响应，合并到缓存。返回是否有新内容。
   *
   * @param silent 静默模式：合并后不触发订阅者通知（供 resolveIdle 批量合并使用）
   */
  private async fetchAndMerge(paths: string[], silent = false): Promise<boolean> {
    let response: NativeIconsResponse
    try {
      response = await invoke<NativeIconsResponse>(CMD_MOLE_NATIVE_ICONS_RESOLVE, { paths })
    } catch (e) {
      console.error('[NativeIconRegistry] resolve failed:', e)
      return false
    }

    let added = false

    // 合并 contentStore（同 contentID 只写一次，幂等）
    for (const [cid, svg] of Object.entries(response.contents)) {
      if (!this.contentStore.has(cid)) {
        this.contentStore.set(cid, svg)
        added = true
      }
    }

    // 合并 pathIndex（同一 path 新 contentID 会覆盖旧条目）
    for (const [path, cid] of Object.entries(response.entries)) {
      const prev = this.pathIndex.get(path)
      if (prev !== cid) {
        this.pathIndex.set(path, cid)
        added = true
      }
    }

    if (added && !silent) this.notify()
    return added
  }

  private notify(): void {
    this._version++
    for (const cb of this.subscribers) {
      try {
        cb()
      } catch (e) {
        console.error('[NativeIconRegistry] subscriber threw:', e)
      }
    }
  }
}

/** 进程内单例。 */
export const nativeIconRegistry = new NativeIconRegistry()
