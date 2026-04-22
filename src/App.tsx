import { useCallback, useEffect, useMemo, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import { EVT_ANALYZE_SCAN_PROGRESS } from '@/constants/tauri-events'
import useTauri from '@/hooks/useTauri'

interface ScanProgressPayload {
  filesScanned: number
  bytesTotal: number
  dirsSeen: number
  lastPath?: string | null
}

interface LargeFile {
  path: string
  size: number
}

interface ScanResult {
  root: string
  filesScanned: number
  dirsSeen: number
  bytesTotal: number
  largestFiles: LargeFile[]
  walkErrors: number
  foldedDirs: number
  foldedBytes: number
}

function formatSize(bytes: number) {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${(bytes / Math.pow(k, i)).toFixed(2)} ${sizes[i]}`
}

export default function App() {
  const tauri = useTauri()
  const [scanRoot, setScanRoot] = useState<string | null>(null)
  const [result, setResult] = useState<ScanResult | null>(null)
  const [progress, setProgress] = useState<ScanProgressPayload | null>(null)
  const [loading, setLoading] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(() => new Set())

  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<ScanProgressPayload>(EVT_ANALYZE_SCAN_PROGRESS, setProgress, ac.signal)
    return () => ac.abort()
  }, [tauri])

  const togglePath = useCallback((path: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  const runScan = async (root: string) => {
    setProgress(null)
    setResult(null)
    setSelected(new Set())
    const data = (await tauri.scan_directory({
      path: root,
      topN: 40,
      progressEvery: 400
    })) as ScanResult
    setResult(data)
  }

  const handlePickAndScan = async () => {
    setLoading(true)
    try {
      const picked = await open({ directory: true, multiple: false })
      if (!picked || typeof picked !== 'string') return
      setScanRoot(picked)
      await runScan(picked)
    } catch (e) {
      console.error(e)
      alert(`扫描失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  const handleRescan = async () => {
    if (!scanRoot) return
    setLoading(true)
    try {
      await runScan(scanRoot)
    } catch (e) {
      alert(`重新扫描失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  const handleTrashSelected = async () => {
    if (!scanRoot || selected.size === 0) return
    if (!confirm(`将 ${selected.size} 个文件移到废纸篓？`)) return
    try {
      await tauri.trash_paths({ root: scanRoot, paths: Array.from(selected) })
      setSelected(new Set())
      alert('已移到废纸篓（可在废纸篓恢复）')
      setLoading(true)
      await runScan(scanRoot)
    } catch (e) {
      alert(`失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  const rows = useMemo(() => result?.largestFiles ?? [], [result])

  return (
    <div style={{ padding: 20, fontFamily: 'system-ui' }}>
      <h1>Mole Desktop — Day1 验证</h1>
      <p style={{ color: '#555', maxWidth: 720 }}>
        选择目录 → 递归扫描（不跟 symlink；完整版对 node_modules 等用 du 折叠）→ Top-N；详细统计见终端日志（RUST_LOG=debug）。
      </p>
      <button type="button" onClick={handlePickAndScan} disabled={loading}>
        {loading ? '扫描中…' : '选择文件夹并扫描'}
      </button>
      {scanRoot && (
        <button type="button" onClick={handleRescan} disabled={loading} style={{ marginLeft: 8 }}>
          重新扫描当前目录
        </button>
      )}
      {scanRoot && (
        <p style={{ marginTop: 12 }}>
          <strong>根目录:</strong> {scanRoot}
        </p>
      )}
      {progress && loading && (
        <p style={{ marginTop: 8, fontSize: 13 }}>
          进度: 已扫文件 {progress.filesScanned}，目录项 {progress.dirsSeen}，累计{' '}
          {formatSize(progress.bytesTotal)}
        </p>
      )}
      {result && (
        <div style={{ marginTop: 16 }}>
          <p>
            共 <strong>{result.filesScanned}</strong> 个文件，总大小约{' '}
            <strong>{formatSize(result.bytesTotal)}</strong>；展示 Top {rows.length} 文件
            {result.foldedDirs > 0 && (
              <>
                {' '}
                · 折叠目录 <strong>{result.foldedDirs}</strong> 个（du 估算{' '}
                {formatSize(result.foldedBytes)}）
              </>
            )}
            {result.walkErrors > 0 && (
              <>
                {' '}
                · walk 错误 <strong>{result.walkErrors}</strong>
              </>
            )}
          </p>
          <button
            type="button"
            disabled={selected.size === 0}
            onClick={handleTrashSelected}
            style={{ marginBottom: 12 }}
          >
            将勾选移到废纸篓 ({selected.size})
          </button>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
            <thead>
              <tr style={{ textAlign: 'left', borderBottom: '1px solid #ccc' }}>
                <th style={{ width: 36 }} />
                <th>路径</th>
                <th style={{ width: 100 }}>大小</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((f) => (
                <tr key={f.path} style={{ borderBottom: '1px solid #eee' }}>
                  <td>
                    <input
                      type="checkbox"
                      checked={selected.has(f.path)}
                      onChange={() => togglePath(f.path)}
                    />
                  </td>
                  <td style={{ wordBreak: 'break-all', paddingRight: 8 }}>{f.path}</td>
                  <td>{formatSize(f.size)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
