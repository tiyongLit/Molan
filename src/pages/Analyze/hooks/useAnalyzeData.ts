import { useState, useCallback, useRef, useEffect } from 'react'
import useTauri, { EVT_ANALYZE_SCAN_PROGRESS } from '@/hooks/useTauri'
import { iconService } from '@/utils/iconService'
import { moleMessage } from '@/components/ui'
import type { UnlistenFn } from '@tauri-apps/api/event'
import type { MoleAnalyzeResult } from '@/types/mole'
import type { BrowseData } from '../typings'

/**
 * 数据获取 Hook — 封装 mole_analyze 调用 + scan-progress 订阅
 *
 * 特性：
 *   - 内存缓存：前进/后退时直接从缓存恢复，不发起请求
 *   - refreshPath：强制刷新（跳过缓存，同时清除过期缓存）
 *   - fetchPath：优先缓存，未命中才请求
 *   - scanProgress：实时扫描进度快照（Rust scanner ~200ms 推送一次）
 *   - re-entrancy guard：同一路径的并发调用只执行一次
 */
export interface ScanProgress {
  files_scanned: number
  dirs_scanned: number
  bytes_scanned: number
  current_path: string
  /** 扫描完成百分比 0–100，-1 表示暂无估算 */
  percent: number
  /** 百分比分母（字节目标）：快照子树大小或卷已用空间；0 = 无 */
  bytes_target: number
  /** 卷总容量（字节，展示用） */
  disk_total: number
}

/** 将 Tauri 返回数据映射为 BrowseData */
function toBrowseData(data: MoleAnalyzeResult): BrowseData {
  return {
    entries: data.entries,
    largeFiles: data.large_files ?? [],
    totalSize: data.total_size,
    totalFiles: data.total_files ?? 0
  }
}

/** 空数据单例 — 避免每次 setBrowseData 创建新对象触发无谓 rerender */
const EMPTY_BROWSEDATA: BrowseData = { entries: [], largeFiles: [], totalSize: 0, totalFiles: 0 }

export function useAnalyzeData() {
  const tauri = useTauri()
  const [browseData, setBrowseData] = useState<BrowseData>(EMPTY_BROWSEDATA)
  const [browseLoading, setBrowseLoading] = useState(false)
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null)
  const cache = useRef<Map<string, BrowseData>>(new Map())
  /** 当前期望的路径：快速连点上下级时，旧请求的响应会被丢弃 */
  const pendingPathRef = useRef<string | null>(null)
  /**
   * 正在加载的路径 — re-entrancy guard：
   * 内部在 await 期间标记，防止 useLayoutEffect 因其他状态变化二次调用导致并发请求。
   */
  const fetchingRef = useRef<string | null>(null)

  // ── 订阅 scan-progress 事件 ──
  useEffect(() => {
    let unlisten: UnlistenFn | undefined

    if (browseLoading) {
      tauri
        .listenIpc<ScanProgress>(EVT_ANALYZE_SCAN_PROGRESS, (payload) => {
          if (pendingPathRef.current) {
            setScanProgress(payload)
          }
        })
        .then((fn) => {
          unlisten = fn
        })
        .catch(() => {})
    }

    return () => {
      unlisten?.()
      setScanProgress(null)
    }
  }, [browseLoading, tauri])

  // 加载完成时清除进度
  useEffect(() => {
    if (!browseLoading) {
      setScanProgress(null)
    }
  }, [browseLoading])

  /**
   * 核心 IPC 调用逻辑 — fetchPath 和 refreshPath 的公共实现。
   * @param skipCache 是否跳过缓存（refreshPath 时为 true）
   */
  const doFetch = useCallback(
    async (path: string, skipCache: boolean) => {
      // ── re-entrancy guard ──
      if (fetchingRef.current === path) {
        return
      }

      pendingPathRef.current = path

      // 缓存命中（仅 fetchPath 模式）
      if (!skipCache) {
        const cached = cache.current.get(path)
        if (cached) {
          if (pendingPathRef.current !== path) return
          setBrowseData(cached)
          setBrowseLoading(false)
          return
        }
      } else {
        // 删除过期缓存
        cache.current.delete(path)
      }

      // 标记加载中，清空旧数据
      fetchingRef.current = path
      setBrowseData(EMPTY_BROWSEDATA)
      setBrowseLoading(true)
      setScanProgress(null)

      try {
        const data = (await tauri.mole_analyze({
          path,
          overview: false,
          skip_cache: skipCache
        })) as MoleAnalyzeResult

        // 竞争保护
        if (pendingPathRef.current !== path) {
          fetchingRef.current = null
          return
        }

        // 预加载图标，确保数据 + native 图标一起渲染
        // 目录 + symlink（symlink 由系统 iconForFile 自带 alias 角标）+ 大文件
        const iconPaths = [
          ...data.entries.filter((e) => e.is_dir || e.is_symlink).map((e) => e.path),
          ...(data.large_files ?? []).map((f) => f.path)
        ]
        if (iconPaths.length > 0) {
          await iconService.preloadIconsIdle(iconPaths)
        }

        const result = toBrowseData(data)
        cache.current.set(path, result)
        setBrowseData(result)
        setBrowseLoading(false)
        fetchingRef.current = null
      } catch (e: unknown) {
        console.error('[doFetch] ERROR:', e)
        fetchingRef.current = null
        if (pendingPathRef.current !== path) return
        moleMessage.error(`扫描失败: ${e instanceof Error ? e.message : String(e)}`)
        setBrowseLoading(false)
      }
    },
    [tauri]
  )

  /** 优先缓存，未命中才请求 */
  const fetchPath = useCallback((path: string) => doFetch(path, false), [doFetch])

  /** 强制刷新（跳过缓存） */
  const refreshPath = useCallback((path: string) => doFetch(path, true), [doFetch])

  return { browseData, browseLoading, scanProgress, fetchPath, refreshPath }
}
