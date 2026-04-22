import { useState, useEffect, useCallback } from 'react'
import { ConfigProvider, theme } from 'antd'
import useTauri from '@/hooks/useTauri'
import { useDiskStatus } from '@/hooks/useDiskStatus'
import { useI18n } from '@/i18n'
import { ShinyText } from '@/components/reactbits'
import { BrowsingView } from './components/BrowsingView'
import { LocationSelector } from './components/LocationSelector'
import type { MoleAnalyzeResult } from '@/types/mole'
import { ScanButton } from '@/components/ui/ScanButton'
import './style.scss'

// ── 页面级 CSS 变量覆盖：轻 scrim 让橙紫渐变透出（对齐 Clean/Uninstall 的浮动式面板语言）──
const ANALYZE_THEME_VARS: React.CSSProperties = {
  '--bg-page': 'rgba(0, 0, 0, 0.14)',
  '--bg-card': 'rgba(0, 0, 0, 0.20)',
  '--border': 'rgba(255, 255, 255, 0.12)',
  '--text-tertiary': 'rgba(255, 255, 255, 0.45)'
} as React.CSSProperties

// ── antd 组件主题：暗色 + 青绿主色（对齐 Uninstall DROPDOWN_THEME，跨路由统一控件语言）──
const ANALYZE_ANTD_THEME = {
  algorithm: theme.darkAlgorithm,
  token: {
    colorBgElevated: 'rgba(28, 28, 40, 0.96)',
    colorText: 'rgba(255,255,255,0.88)',
    colorPrimary: '#64dfa7',
    colorPrimaryHover: '#7ff0bd'
  }
}

type Phase = 'overview' | 'browsing'

/**
 * Analyze 页面 — 两阶段状态机
 *   overview：选择文件夹 + 开始分析
 *   browsing：列表 + Treemap + 废纸篓（BrowsingView）
 */
export function Analyze() {
  const tauri = useTauri()
  const { disk } = useDiskStatus(tauri)
  const { t } = useI18n()
  const [phase, setPhase] = useState<Phase>('overview')
  const [overviewResult, setOverviewResult] = useState<MoleAnalyzeResult | null>(null)
  const [overviewError, setOverviewError] = useState<string | null>(null)
  const [selectedPath, setSelectedPath] = useState('/')
  // browsingKey 递增 → 强制重建 BrowsingView（切换根目录时清空内部状态）
  const [browsingKey, setBrowsingKey] = useState(0)

  // ── 加载 overview（对齐 V1：空 path ⇒ 后端视为全机 overview） ──
  // 按钮不依赖此状态：仅作为「概览数据」在 LocationSelector 内（根目录的容量进度）使用。
  // 若加载失败，按钮仍可点击（用户可从 '/' 或 '$HOME' 入口直接进 browsing 模式）。
  useEffect(() => {
    let cancelled = false
    setOverviewError(null)

    tauri
      .mole_analyze({ path: '', overview: true })
      .then((data: MoleAnalyzeResult) => {
        if (cancelled) return
        setOverviewResult(data)
      })
      .catch((e: unknown) => {
        if (cancelled) return
        setOverviewError(e instanceof Error ? e.message : String(e))
      })

    return () => {
      cancelled = true
    }
  }, [tauri])

  // ── 开始分析 ──
  const handleStartScan = useCallback(() => {
    if (!selectedPath) return
    setBrowsingKey((k) => k + 1)
    setPhase('browsing')
  }, [selectedPath])

  // ── 重新开始 ──
  const handleBackToOverview = useCallback(() => {
    setPhase('overview')
  }, [])

  // ── 切换根目录（面包屑首项下拉） ──
  const handleSwitchRoot = useCallback((path: string) => {
    setSelectedPath(path)
    setBrowsingKey((k) => k + 1)
  }, [])

  if (phase === 'browsing' && overviewResult) {
    return (
      <ConfigProvider theme={ANALYZE_ANTD_THEME}>
        <div className="h-full" style={ANALYZE_THEME_VARS}>
          <BrowsingView
            key={browsingKey}
            initialPath={selectedPath}
            overviewResult={overviewResult}
            onBackToOverview={handleBackToOverview}
            onSwitchRoot={handleSwitchRoot}
          />
        </div>
      </ConfigProvider>
    )
  }

  return (
    <ConfigProvider theme={ANALYZE_ANTD_THEME}>
      <div className="relative flex min-h-full w-full items-center justify-center" style={ANALYZE_THEME_VARS}>
        <div className="flex flex-col gap-6 items-center" style={{ minWidth: 360, maxWidth: 440 }}>
          <div className="flex flex-col gap-2">
            <ShinyText
              text={t('analyze.title')}
              speed={2}
              delay={0}
              color="#ffffff"
              shineColor="#b5b5b5"
              spread={120}
              direction="left"
              yoyo={false}
              pauseOnHover={false}
              disabled={false}
              className="text-3xl font-semibold tracking-tight"
            />
             <p className="mt-2 text-sm text-[var(--text-secondary)]">
            {t('analyze.subtitle')}
          </p>
          </div>

          <LocationSelector
            value={selectedPath}
            onChange={setSelectedPath}
            disk={disk}
          />
          {/* ── 按钮外置（参照 V1 Analyze：按钮与卡片同级） ── */}
          <div className="flex flex-col items-center gap-2.5 w-full">
            <ScanButton
              type="primary"
              size="small"
              scanEffect
              block
              disabled={!selectedPath}
              onClick={handleStartScan}
              style={{ color: 'var(--accent)', borderRadius: 8 }}
            >
              {t('analyze.startScan')}
            </ScanButton>
          </div>

          {/* ── overview 错误：以次要文字提示，不阻塞按钮 ── */}
          {overviewError && (
            <p className="text-[10px] text-red-400/70 text-center max-w-[360px] -mt-4">
              {t('analyze.overviewFailed', { error: overviewError })}
            </p>
          )}

        </div>
      </div>
    </ConfigProvider>
  )
}
