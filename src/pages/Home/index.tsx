import { useCallback, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import useTauri from '@/hooks/useTauri'
import { useDiskStatus } from '@/hooks/useDiskStatus'
import { DashboardDiskCard } from '@/components/business/DiskCard'
import { ShinyText } from '@/components/reactbits'
import { ScanButton } from '@/components/ui/ScanButton'
import { useScanButton } from '@/layout/ScanButtonContext'
import HeroIllustration from './HeroIllustration'
import { FdaBanner } from './FdaBanner'
import { uiTrace } from '@/utils/uiTrace'
import { useI18n } from '@/i18n'

export function Home() {
  const navigate = useNavigate()
  const tauri = useTauri()
  const { disk, volumeIconSrc, openStorageSettingsSupported } = useDiskStatus(tauri)
  const { setState: setScanButton } = useScanButton()
  const { t, locale } = useI18n()

  // 立即扫描：仅负责跳转 + 传递 state.autoScan=true
  // admin session 申请交给 Clean 页面 autoScan effect 统一处理（避免 Home→Clean 路径的 IPC 时序 race：
  // 在 Home 申请 → resolve 时 navigate → Clean 立即调 clean_scan → 后端 session 尚未就绪 → 报错回 idle）
  const handleStartScan = useCallback(() => {
    // 时序埋点（卡顿分析）：首页「立即扫描」点击点（跳 Clean + autoScan 的入口）
    uiTrace('home.scan.click')
    navigate('/clean', { state: { autoScan: true } })
  }, [navigate])

  useEffect(() => {
    setScanButton({
      visible: true,
      percent: 0,
      label: t('home.scan'),
      strokeColor: '#ff9448',
      onClick: handleStartScan,
    })
    return () => setScanButton({ visible: false })
  }, [handleStartScan, setScanButton, t, locale])

  const accentColor = 'var(--accent)'

  return (
    <div className="relative flex min-h-full w-full items-center justify-center">
      {/* ── 背景装饰：Hero 插图左下错位 ── */}
      <div className="absolute bottom-0 left-4 opacity-12 pointer-events-none">
        <HeroIllustration className="h-60 w-70" />
      </div>

      {/* ── FDA 权限软提示横幅：绝对定位于内容区顶部（不进居中列，出现/让位零位移） ── */}
      <div className="absolute top-24 left-1/2 z-20 w-full max-w-[440px] -translate-x-1/2">
        <FdaBanner />
      </div>

      {/* ── 居中：标题 + 磁盘卡片 ── */}
      <div className="flex flex-col gap-6 items-center" style={{ minWidth: 360, maxWidth: 440 }}>
        <div className="flex flex-col gap-2">
          <ShinyText
            text="Molan"
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
          <p
            className="text-sm leading-relaxed text-[var(--text-secondary)]"
          >
            {t('home.tagline')}
          </p>
        </div>

        <DashboardDiskCard
          disk={disk}
          volumeIconSrc={volumeIconSrc}
          openStorageSettingsSupported={openStorageSettingsSupported}
          accentColor={accentColor}
        />

        {/* ── 卡片外 CTA（参照 V1 Home 布局：按钮与卡片同级）── */}
        <ScanButton
          type="primary"
          size="small"
          scanEffect
          block
          onClick={handleStartScan}
          style={{ color: accentColor, borderRadius: 8 }}
        >
          {t('home.scanNow')}
        </ScanButton>
      </div>
    </div>
  )
}
