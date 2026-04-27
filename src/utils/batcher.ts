/**
 * 轻量级 Batcher：同一 key 的并发 invoke 合并为一次 IPC 调用。
 *
 * 从 Next.js Batcher 简化而来，仅处理字符串 key、同步调度（无 scheduler 依赖）。
 *
 * 用法：
 *   const batcher = new Batcher<any>()
 *   const data = await batcher.batch('myKey', () => fetch('/api'))
 */
export class Batcher<V> {
  private readonly pending = new Map<string, Promise<V>>()

  /**
   * 批处理入口：
   * - 如果 key 对应的调用已在执行中，直接返回已有的 Promise（不重复执行）
   * - 否则执行 fn 并缓存结果 Promise
   * - 无论成功/失败，调用完成后自动清除缓存，下次相同 key 会重新执行
   */
  async batch(key: string, fn: () => Promise<V>): Promise<V> {
    const existing = this.pending.get(key)
    if (existing) return existing

    const promise = fn().finally(() => {
      this.pending.delete(key)
    })
    this.pending.set(key, promise)
    return promise
  }

  /** 当前 pending 中的 key 数量（调试用） */
  get size(): number {
    return this.pending.size
  }

  /** 清空所有 pending 调用（通常在组件卸载等场景使用） */
  clear(): void {
    this.pending.clear()
  }
}
