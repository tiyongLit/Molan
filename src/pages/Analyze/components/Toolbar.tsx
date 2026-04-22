import { useCallback } from 'react'
import { CaretLeftOutlined, CaretRightOutlined, SyncOutlined } from '@ant-design/icons'
import { MoleButton } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { useAnalyzeNav } from '../contexts/AnalyzeNavContext'
import { useAnalyzeData } from '../contexts/AnalyzeDataContext'
import { useAnalyzeAction } from '../contexts/AnalyzeActionContext'
import { useI18n } from '@/i18n'
import { PathBreadcrumb } from './PathBreadcrumb'
import { Undo2 } from 'lucide-react'

const NAV_BTN_STYLE: React.CSSProperties = {
  width: 24,
  height: 24,
  padding: 0,
  borderRadius: 2,
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center'
}

/**
 * 顶部工具栏（V2 单排布局）：
 *   左：后退/前进 + 面包屑导航
 *   右：重新扫描按钮
 */
export function Toolbar() {
  const nav = useAnalyzeNav()
  const { browseLoading, cancelling } = useAnalyzeData()
  const { refreshPath, cancelScanAndExit } = useAnalyzeAction()
  const { t } = useI18n()

  // 返回：扫描中走取消+退出（与 ScanOverlay 取消按钮同底层），否则直接返回 overview
  const handleBack = useCallback(() => {
    if (browseLoading || cancelling) {
      void cancelScanAndExit()
    } else {
      nav.backToOverview()
    }
  }, [browseLoading, cancelling, cancelScanAndExit, nav.backToOverview])

  // 返回前原生确认（NSAlert 垂直三行流）：取消＝关闭对话框什么都不做；确认＝真正返回。
  const onBackClick = useCallback(async () => {
    const ok = await moleNativeConfirm(t('analyze.toolbar.exitConfirm'))
    if (ok) handleBack()
  }, [handleBack, t])

  return (
    <>
      <div className="flex items-center gap-1 pr-[12px] h-[38px] shrink-0">
        <MoleButton
          type="text"
          size="small"
          icon={<Undo2 size={14} />}
          title={t('analyze.toolbar.back')}
          onClick={() => {
            void onBackClick()
          }}
        />


        <MoleButton
          type="text"
          size="small"
          icon={<CaretLeftOutlined />}
          onClick={nav.canGoBack ? nav.goBack : nav.backToOverview}
          title={nav.canGoBack ? t('analyze.toolbar.prev') : t('analyze.toolbar.restart')}
          style={NAV_BTN_STYLE}
          className="analyze-nav-btn"
        />
        <MoleButton
          type="text"
          size="small"
          icon={<CaretRightOutlined />}
          disabled={!nav.canGoForward}
          onClick={nav.goForward}
          title={t('analyze.toolbar.next')}
          style={NAV_BTN_STYLE}
          className="analyze-nav-btn"
        />

        <div className="flex-1 min-w-0 mx-1">
          <PathBreadcrumb />
        </div>

        <MoleButton
          type="text"
          size="small"
          icon={<SyncOutlined spin={browseLoading} />}
          // 扫描中禁用：同路径重扫会撞上 doFetch 的 re-entrancy guard 而形成「点了没反应」；
          // 此时取消入口在进度浮层上（取消即返回 overview）。
          disabled={browseLoading || cancelling}
          onClick={() => refreshPath(nav.currentPath)}
          className="!h-6 !px-2 !rounded-[3px] !border !border-white/[0.12] !bg-white/[0.08] hover:!bg-white/[0.12] !text-white/70 hover:!text-white/85 analyze-toggle-btn shrink-0"
          style={{ fontSize: 11 }}
        >
          {t('analyze.toolbar.rescan')}
        </MoleButton>
      </div>

      {/* hairline 分隔线（对齐 Uninstall：顶部条无背景，直接浮在渐变上） */}
      <div className="shrink-0 h-px bg-white/[0.10]" />
    </>
  )
}
