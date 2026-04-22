import { useEffect } from 'react'
import { ConfigProvider, theme } from 'antd'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { GeneralSection } from './sections/GeneralSection'
import { useSettings } from './useSettings'
import { cmmPalette } from '@/layout/themeColors'
import { useI18n } from '@/i18n'

// ── 主题色（对齐 Clean 页方案：深藏青基调，文字对比度优于 dashboard 亮青） ──

const palette = cmmPalette.clean
const [br, bg, bb] = palette.bloom
const [dr, dg, db] = palette.deep
const pageBg = `linear-gradient(160deg, rgb(${br},${bg},${bb}) 0%, rgb(${dr},${dg},${db}) 100%)`

// ── 主组件（扁平布局：无分组标题，父子项合并为视觉块，间距对齐柠檬 20px 节奏） ──

export function Settings() {
  const { settings, loading, trashSaving, trashError, updateSetting, toggleAutoLaunch, changeLanguage } = useSettings()
  const { t } = useI18n()

  // 透明窗口需要 body / #root 背景透明（对齐 Dashboard / MainLayout）
  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.body.style.backgroundColor = 'transparent'
    const root = document.getElementById('root')
    if (root) {
      root.style.overflow = 'visible'
      root.style.borderRadius = '0'
      root.style.background = 'transparent'
    }
  }, [])

  // 窗口拖拽（overlay titlebar 区域）
  const handleMouseDown = () => {
    getCurrentWindow().startDragging()
  }

  return (
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          // 主题色一致的深藏青（MAIL_DEEP），让 antd 所有浮层（Tooltip、Select、Popover）
          // 与页面渐变同色相，避免默认冷黑蓝突兀。
          colorBgElevated: 'rgba(53,64,112,0.95)',
          borderRadius: 6,
          colorSplit: 'rgba(255,255,255,0.10)',
        },
      }}
    >
      <div
        className="settings-page flex flex-col w-full h-full select-none"
        style={{ background: pageBg }}
        onMouseDown={handleMouseDown}
      >
        {/* ── 主体：单列滚动（overlay titlebar 提供窗口标题）；px-7 对齐柠檬 28px 内容左边距 ── */}
        <div className="flex-1 min-h-0 overflow-y-auto px-7 pt-12 pb-4">
          {loading ? (
            <div className="text-[12px] text-white/40 pt-4">{t('settings.loading')}</div>
          ) : (
            <GeneralSection
              settings={settings}
              trashSaving={trashSaving}
              trashError={trashError}
              onToggleAutoLaunch={toggleAutoLaunch}
              onChangeLanguage={changeLanguage}
              onUpdateSetting={updateSetting}
            />
          )}
        </div>
      </div>
    </ConfigProvider>
  )
}
