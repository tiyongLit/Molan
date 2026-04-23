import { useCallback, useEffect, useMemo, useState } from 'react'
import { Button, Collapse, ConfigProvider, Radio, Space, Tree, Typography, theme } from 'antd'
import type { RadioChangeEvent } from 'antd/es/radio'
import type { DataNode } from 'antd/es/tree'

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

interface ScanNode {
  path: string
  name: string
  size: number
  isDir: boolean
  isFolded: boolean
  children: ScanNode[]
}

interface RuleItemBlueprint {
  id: string
  title: string
  tips: string
  recommend: boolean
  cautious: boolean
  relativePaths: string[]
  /** 动态规则命中本机已装应用的 Bundle ID */
  matchedBundleId?: string
}

interface RuleCategoryBlueprint {
  id: string
  title: string
  tips: string
  items: RuleItemBlueprint[]
}

interface ScanResult {
  root: string
  filesScanned: number
  dirsSeen: number
  bytesTotal: number
  largestFiles: LargeFile[]
  items: ScanNode[]
  walkErrors: number
  foldedDirs: number
  foldedBytes: number
  /** `logical` | `physical` */
  sizeMetric: string
  /** 编译期内嵌的 Lemon 式分类骨架（MAS 友好） */
  ruleCategories: RuleCategoryBlueprint[]
}

const SCAN_TOP_N = 40
const SCAN_PROGRESS_EVERY = 400

type SizeMetricMode = 'logical' | 'physical'

function formatSize(bytes: number) {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${(bytes / Math.pow(k, i)).toFixed(2)} ${sizes[i]}`
}

function collectFileLeafPaths(nodes: ScanNode[]): Set<string> {
  const out = new Set<string>()
  const walk = (list: ScanNode[]) => {
    for (const n of list) {
      if (!n.isDir) {
        out.add(n.path)
      } else if (n.children?.length) {
        walk(n.children)
      }
    }
  }
  walk(nodes)
  return out
}

function buildPathSizeMap(nodes: ScanNode[]): Map<string, number> {
  const m = new Map<string, number>()
  const walk = (list: ScanNode[]) => {
    for (const n of list) {
      if (!n.isDir) {
        m.set(n.path, n.size)
      }
      if (n.children?.length) walk(n.children)
    }
  }
  walk(nodes)
  return m
}

function buildTreeData(nodes: ScanNode[]): DataNode[] {
  return nodes.map((n) => ({
    key: n.path,
    title: (
      <span
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'flex-start',
          gap: 12,
          width: '100%',
          paddingRight: 8
        }}
      >
        <Typography.Text ellipsis={{ tooltip: n.path }} style={{ flex: 1, marginBottom: 0 }}>
          {n.isFolded ? `${n.name}` : n.name}
        </Typography.Text>
        <Typography.Text type="secondary" style={{ flexShrink: 0, marginBottom: 0 }}>
          {formatSize(n.size)}
        </Typography.Text>
      </span>
    ),
    isLeaf: !n.isDir,
    children: n.children?.length ? buildTreeData(n.children) : undefined
  }))
}

export default function App() {
  const tauri = useTauri()
  const [scanRoot, setScanRoot] = useState<string | null>(null)
  const [result, setResult] = useState<ScanResult | null>(null)
  const [progress, setProgress] = useState<ScanProgressPayload | null>(null)
  const [loading, setLoading] = useState(false)
  /** 仅文件路径（可移入废纸篓） */
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(() => new Set())
  const [checkedKeys, setCheckedKeys] = useState<React.Key[]>([])
  const [sizeMetric, setSizeMetric] = useState<SizeMetricMode>('logical')

  useEffect(() => {
    const ac = new AbortController()
    tauri.onIpcEvent<ScanProgressPayload>(EVT_ANALYZE_SCAN_PROGRESS, setProgress, ac.signal)
    return () => ac.abort()
  }, [tauri])

  const leafPathSet = useMemo(
    () => (result?.items?.length ? collectFileLeafPaths(result.items) : new Set<string>()),
    [result]
  )

  const pathSizeMap = useMemo(
    () => (result?.items?.length ? buildPathSizeMap(result.items) : new Map<string, number>()),
    [result]
  )

  const treeData = useMemo(
    () => (result?.items?.length ? buildTreeData(result.items) : []),
    [result]
  )

  const selectedBytes = useMemo(() => {
    let t = 0
    for (const p of selectedFiles) {
      t += pathSizeMap.get(p) ?? 0
    }
    return t
  }, [selectedFiles, pathSizeMap])

  const onCheck = useCallback(
    (checked: React.Key[] | { checked: React.Key[]; halfChecked: React.Key[] }) => {
      const keys = Array.isArray(checked) ? checked : checked.checked
      setCheckedKeys(keys)
      const next = new Set<string>()
      for (const k of keys) {
        const s = String(k)
        if (leafPathSet.has(s)) next.add(s)
      }
      setSelectedFiles(next)
    },
    [leafPathSet]
  )

  const runScanHome = useCallback(async () => {
    setProgress(null)
    setResult(null)
    setSelectedFiles(new Set())
    setCheckedKeys([])
    const data = (await tauri.scan_home({
      topN: SCAN_TOP_N,
      progressEvery: SCAN_PROGRESS_EVERY,
      sizeMetric
    })) as ScanResult
    setScanRoot(data.root)
    setResult(data)
  }, [tauri, sizeMetric])

  const onSizeMetricChange = (e: RadioChangeEvent) => {
    const v = e.target.value as SizeMetricMode
    setSizeMetric(v)
  }

  const handleScanMyMac = async () => {
    setLoading(true)
    try {
      await runScanHome()
    } catch (e) {
      console.error(e)
      alert(`扫描失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  const handleRescan = async () => {
    setLoading(true)
    try {
      await runScanHome()
    } catch (e) {
      alert(`重新扫描失败: ${e}`)
    } finally {
      setLoading(false)
    }
  }

  const handleTrashSelected = async () => {
    if (!scanRoot || selectedFiles.size === 0) return
    if (!confirm(`将 ${selectedFiles.size} 个文件移到废纸篓？`)) return
    try {
      await tauri.trash_paths({ root: scanRoot, paths: Array.from(selectedFiles) })
      setSelectedFiles(new Set())
      setCheckedKeys([])
      alert('已移到废纸篓（可在废纸篓恢复）')
      setLoading(true)
      try {
        await runScanHome()
      } finally {
        setLoading(false)
      }
    } catch (e) {
      alert(`失败: ${e}`)
    }
  }

  return (
    <ConfigProvider theme={{ algorithm: theme.defaultAlgorithm }}>
      <div style={{ padding: 20, maxWidth: 960, margin: '0 auto' }}>
        <Typography.Title level={3} style={{ marginTop: 0 }}>
          Mole Desktop
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
          一键扫描当前用户主目录（与 Lemon / CleanMyMac 式入口一致）。官网完整版可透视真实{' '}
          <Typography.Text code>~</Typography.Text>；App Store 精简版使用同一入口，可读范围受沙箱限制。扫描不跟随符号链接；折叠目录为纯
          Rust 统计，可展开查看子树 Top-N。仅文件可移入废纸篓。
        </Typography.Paragraph>
        <div style={{ marginBottom: 16 }}>
          <Typography.Text strong style={{ marginRight: 12 }}>
            统计口径
          </Typography.Text>
          <Radio.Group value={sizeMetric} onChange={onSizeMetricChange}>
            <Radio.Button value="logical">逻辑大小（对标 Lemon / Finder）</Radio.Button>
            <Radio.Button value="physical">物理占用（对标 Mole）</Radio.Button>
          </Radio.Group>
          <Typography.Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0, fontSize: 13 }}>
            切换口径后请点击「扫描我的 Mac」或「重新扫描」刷新结果。
          </Typography.Paragraph>
        </div>
        <Space wrap>
          <Button type="primary" size="large" onClick={handleScanMyMac} loading={loading}>
            扫描我的 Mac
          </Button>
          {scanRoot && (
            <Button size="large" onClick={handleRescan} loading={loading}>
              重新扫描
            </Button>
          )}
        </Space>
        {scanRoot && (
          <Typography.Paragraph style={{ marginTop: 12, marginBottom: 0 }}>
            <Typography.Text strong>扫描根目录: </Typography.Text>
            <Typography.Text code>{scanRoot}</Typography.Text>
          </Typography.Paragraph>
        )}
        {progress && loading && (
          <Typography.Paragraph type="secondary" style={{ marginTop: 8, fontSize: 13 }}>
            进度: 已扫文件 {progress.filesScanned}，目录项 {progress.dirsSeen}，累计{' '}
            {formatSize(progress.bytesTotal)}
          </Typography.Paragraph>
        )}
        {result && (
          <div style={{ marginTop: 20 }}>
            <Space direction="vertical" size="middle" style={{ width: '100%' }}>
              <Typography.Title level={5} style={{ marginBottom: 8 }}>
                清理分类（规则内嵌于应用，无外部 rules.yaml）
              </Typography.Title>
              <Collapse
                size="small"
                items={result.ruleCategories.map((cat) => ({
                  key: cat.id,
                  label: (
                    <span>
                      <Typography.Text strong>{cat.title}</Typography.Text>
                      <Typography.Text type="secondary" style={{ marginLeft: 8, fontSize: 12 }}>
                        {cat.tips}
                      </Typography.Text>
                    </span>
                  ),
                  children: (
                    <ul style={{ margin: 0, paddingLeft: 20 }}>
                      {cat.items.map((it) => (
                        <li key={it.id} style={{ marginBottom: 8 }}>
                          <Typography.Text strong>{it.title}</Typography.Text>
                          {it.cautious && (
                            <Typography.Text type="warning" style={{ marginLeft: 6, fontSize: 12 }}>
                              谨慎
                            </Typography.Text>
                          )}
                          {!it.recommend && !it.cautious && (
                            <Typography.Text type="secondary" style={{ marginLeft: 6, fontSize: 12 }}>
                              默认不勾选
                            </Typography.Text>
                          )}
                          <div>
                            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                              {it.tips}
                            </Typography.Text>
                          </div>
                          <Typography.Paragraph
                            type="secondary"
                            style={{ fontSize: 11, marginBottom: 0, wordBreak: 'break-all' }}
                          >
                            {it.relativePaths.join(' · ')}
                          </Typography.Paragraph>
                          {it.matchedBundleId && (
                            <Typography.Paragraph
                              type="secondary"
                              style={{ fontSize: 11, marginBottom: 0, wordBreak: 'break-all' }}
                            >
                              Bundle ID: <Typography.Text code>{it.matchedBundleId}</Typography.Text>
                            </Typography.Paragraph>
                          )}
                        </li>
                      ))}
                    </ul>
                  )
                }))}
              />
              <Typography.Title level={5} style={{ marginTop: 8, marginBottom: 0 }}>
                整盘扫描结果（Top-N 与折叠目录）
              </Typography.Title>
              <Typography.Text>
                口径: <strong>{result.sizeMetric === 'physical' ? '物理占用 (Mole)' : '逻辑大小 (Lemon/Finder)'}</strong>
                {' · '}
                共 <strong>{result.filesScanned}</strong> 个文件，总大小约{' '}
                <strong>{formatSize(result.bytesTotal)}</strong>
                {result.foldedDirs > 0 && (
                  <>
                    {' '}
                    · 折叠目录 <strong>{result.foldedDirs}</strong> 个（
                    {formatSize(result.foldedBytes)}）
                  </>
                )}
                {result.walkErrors > 0 && (
                  <>
                    {' '}
                    · walk 错误 <strong>{result.walkErrors}</strong>
                  </>
                )}
              </Typography.Text>
              <div
                style={{
                  display: 'flex',
                  flexWrap: 'wrap',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12
                }}
              >
                <Typography.Text>
                  已选: <strong>{formatSize(selectedBytes)}</strong>（{selectedFiles.size} 个文件）
                </Typography.Text>
                <Button
                  type="primary"
                  danger
                  disabled={selectedFiles.size === 0}
                  onClick={handleTrashSelected}
                >
                  立即清理（废纸篓）
                </Button>
              </div>
              {treeData.length > 0 ? (
                <Tree
                  checkable
                  blockNode
                  showLine
                  defaultExpandAll={false}
                  treeData={treeData}
                  checkedKeys={checkedKeys}
                  onCheck={onCheck}
                  style={{ background: '#fafafa', padding: 12, borderRadius: 8 }}
                />
              ) : (
                <Typography.Text type="secondary">
                  当前可见范围内无展示项（空目录、权限不足或沙箱未授权路径）。完整透视请使用官网分发版本。
                </Typography.Text>
              )}
            </Space>
          </div>
        )}
      </div>
    </ConfigProvider>
  )
}
