import { useState, useCallback, useRef, useEffect } from 'react'
import { throttle } from 'lodash-es'
import useTauri, { EVT_ANALYZE_SCAN_PROGRESS } from '@/hooks/useTauri'
import { nativeIconRegistry } from '@/utils/nativeIconRegistry'
import { moleMessage } from '@/components/ui'
import { t } from '@/i18n'
import type { MoleAnalyzeResult } from '@/types/mole'
import type { BrowseData } from '../typings'

/**
 * 数据获取 Hook — 「一次扫描 + 内存导航」模型（对齐 Lemon Cleaner）。
 *
 * 核心设计:
 *   - scanRoot(path): 首次全量扫描（重型，显示进度条），结果存入后端内存会话树
 *   - navigateTo(path): 后续目录导航（轻型，微秒级），直接读后端内存树
 *   - 无前端缓存 Map、无竞态归属判定——导航是同步级操作，不存在并发问题
 *   - bundle 叶子钻取: navigate miss → fallback 到 mole_analyze 做 scoped scan
 *   - 取消: 仅首次扫描需要（导航是瞬时的，无需取消）
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

/** 后端取消错误码 */
const SCAN_CANCELLED_CODE = 'SCAN_CANCELLED'
/** 后端导航 miss 标识（路径不在会话树中） */
const NAVIGATE_MISS = 'NAVIGATE_MISS'

function isScanCancelled(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : String(e)
  return msg === SCAN_CANCELLED_CODE || msg.includes('scan cancelled')
}

function isNavigateMiss(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : String(e)
  return msg === NAVIGATE_MISS || msg.includes(NAVIGATE_MISS)
}

export function useAnalyzeData() {
  const tauri = useTauri()
  const [browseData, setBrowseData] = useState<BrowseData>(EMPTY_BROWSEDATA)
  const [browseLoading, setBrowseLoading] = useState(false)
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null)
  const [cancelling, setCancelling] = useState(false)

  /** 扫描是否已完成（首次 scanRoot 成功后为 true，后续导航不需要进度条） */
  const sessionReady = useRef(false)
  /** 当前正在执行的扫描请求 id（仅用于取消判定） */
  const scanReqRef = useRef(0)

  // ── 订阅 scan-progress 事件（仅首次扫描期间）──
  // 后端可能高频推送（每秒几十次），每次都 setState 会让 AnalyzeProvider
  // 重渲染 → 所有 Context 消费者跟随重渲染。节流到 ~10fps（100ms）在
  // 保证进度条跟手的同时大幅减少重渲染。trailing 保证最后一次进度不丢。
  useEffect(() => {
    if (!browseLoading) return
    const ac = new AbortController()
    const throttledSet = throttle(
      (payload: ScanProgress) => setScanProgress(payload),
      100,
      { leading: true, trailing: true }
    )
    tauri.onIpcEvent<ScanProgress>(
      EVT_ANALYZE_SCAN_PROGRESS,
      throttledSet,
      ac.signal
    )
    return () => {
      ac.abort()
      throttledSet.cancel()
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
   * 首次全量扫描（重型）：调用 mole_analyze，后端扫描完整棵树并存入内存会话树。
   * 仅在用户点击"开始分析"时调用一次。
   */
  const scanRoot = useCallback(
    async (path: string) => {
      const reqId = ++scanReqRef.current
      sessionReady.current = false
      setBrowseData(EMPTY_BROWSEDATA)
      setBrowseLoading(true)
      setScanProgress(null)
      setCancelling(false)

      try {
        const data = (await tauri.mole_analyze({
          path,
          overview: false,
          skip_cache: false
        })) as MoleAnalyzeResult

        // 被取消或被新扫描取代
        if (scanReqRef.current !== reqId) return

        sessionReady.current = true
        const result = toBrowseData(data)
        setBrowseData(result)
        setBrowseLoading(false)
        setCancelling(false)

        // 后台预加载图标（不阻塞渲染；写入注册表后由 useNativeIconMap 订阅自动刷新，
        // 全部命中时不产生任何重渲染）
        const iconPaths = [
          ...data.entries.filter((e) => e.is_dir || e.is_symlink).map((e) => e.path),
          ...(data.large_files ?? []).map((f) => f.path)
        ]
        if (iconPaths.length > 0) {
          nativeIconRegistry.resolveIdle(iconPaths).catch(() => {})
        }
      } catch (e: unknown) {
        if (scanReqRef.current !== reqId) return
        setCancelling(false)
        if (isScanCancelled(e)) {
          setBrowseData(EMPTY_BROWSEDATA)
          setBrowseLoading(false)
          return
        }
        console.error('[scanRoot] ERROR:', e)
        moleMessage.error(
          t('analyze.error.scanFailed', { error: e instanceof Error ? e.message : String(e) })
        )
        setBrowseLoading(false)
      }
    },
    [tauri]
  )

  /**
   * 目录导航（轻型）：从后端内存会话树读取，微秒级返回。
   * 所有 drillIn / goBack / breadcrumbJump 都走此路径。
   *
   * 始终先尝试 navigate：
   * - 命中：直接返回（包括组件重挂载后端 session 仍在的情况）
   * - MISS：fallback 到 scanRoot 做全量扫描
   */
  const navigateTo = useCallback(
    async (path: string) => {
      try {
        const data = (await tauri.mole_analyze_navigate({ path })) as MoleAnalyzeResult
        // navigate 不回传 large_files（Top20 是全树固定数据，scanRoot 时已拿到），
        // 保留上一轮的 largeFiles，省去后端每次导航的 clone + 序列化。
        setBrowseData((prev) => ({
          entries: data.entries,
          largeFiles: prev.largeFiles,
          totalSize: data.total_size,
          totalFiles: data.total_files ?? 0
        }))
        sessionReady.current = true

        // 后台预加载图标（仅 miss 项发 IPC；写入注册表后由订阅自动刷新）
        const iconPaths = data.entries
          .filter((e) => e.is_dir || e.is_symlink)
          .map((e) => e.path)
        if (iconPaths.length > 0) {
          nativeIconRegistry.resolveIdle(iconPaths).catch(() => {})
        }
      } catch (e: unknown) {
        if (isNavigateMiss(e)) {
          // 会话树未建立或路径不在树中（bundle 叶子钻取）→ 全量扫描
          return scanRoot(path)
        }
        console.error('[navigateTo] ERROR:', e)
        moleMessage.error(
          t('analyze.error.navFailed', { error: e instanceof Error ? e.message : String(e) })
        )
      }
    },
    [tauri, scanRoot]
  )

  /**
   * 兼容旧 API：fetchPath 现在直接走 navigateTo。
   * AnalyzeContext 中的 useLayoutEffect 依赖此函数名。
   */
  const fetchPath = useCallback(
    (path: string) => {
      void navigateTo(path)
    },
    [navigateTo]
  )

  /**
   * 强制重扫（删除文件后刷新 / 用户手动刷新）：
   * 清除会话树，重新全量扫描当前路径。
   */
  const refreshPath = useCallback(
    async (path: string) => {
      sessionReady.current = false
      return scanRoot(path)
    },
    [scanRoot]
  )

  /**
   * 取消当前扫描。
   */
  const cancelScan = useCallback(async () => {
    if (!browseLoading) return
    setCancelling(true)
    try {
      await tauri.mole_analyze_cancel()
    } catch {
      // 取消请求失败也不阻塞
    }
  }, [tauri, browseLoading])

  /**
   * 释放后端内存会话树（离开 Analyze 页面时调用）。
   */
  const clearSession = useCallback(() => {
    sessionReady.current = false
    tauri.mole_analyze_clear_session().catch(() => {})
  }, [tauri])

  return {
    browseData,
    browseLoading,
    scanProgress,
    cancelling,
    fetchPath,
    refreshPath,
    cancelScan,
    clearSession,
    scanRoot
  }
}
