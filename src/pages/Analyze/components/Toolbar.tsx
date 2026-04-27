import { useCallback } from 'react'
import { CaretLeftOutlined, CaretRightOutlined, SyncOutlined } from '@ant-design/icons'
import { MoleButton } from '@/components/ui'
import { useMoleConfirm } from '@/hooks/useMoleConfirm'
import { useAnalyze } from '../contexts/AnalyzeContext'
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
  const ctx = useAnalyze()
  const { currentPath, refreshPath, browseLoading } = ctx
  const moleConfirm = useMoleConfirm()

  // 返回：原生确认框，确认后重置扫描结果
  const handleBack = useCallback(async () => {
    const confirmed = await moleConfirm('你是否要重置当前扫描结果并重新开始？', {
      title: '重新开始',
      kind: 'warning',
      okLabel: '确认',
      cancelLabel: '取消',
    })
    if (confirmed) ctx.backToOverview()
  }, [ctx, moleConfirm])

  return (
    <>
      <div className="flex items-center gap-1 pr-[12px] h-[38px] shrink-0">
        <MoleButton
          type="text"
          size="small"
          icon={<Undo2 size={14} />}
          title="返回"
          onClick={handleBack}
        />


        <MoleButton
          type="text"
          size="small"
          icon={<CaretLeftOutlined />}
          onClick={ctx.canGoBack ? ctx.goBack : ctx.backToOverview}
          title={ctx.canGoBack ? '后退' : '重新开始'}
          style={NAV_BTN_STYLE}
          className="analyze-nav-btn"
        />
        <MoleButton
          type="text"
          size="small"
          icon={<CaretRightOutlined />}
          disabled={!ctx.canGoForward}
          onClick={ctx.goForward}
          title="前进"
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
          onClick={() => refreshPath(currentPath)}
          className="!h-6 !px-2 !rounded-[3px] !border !border-white/[0.12] !bg-white/[0.08] hover:!bg-white/[0.12] !text-white/70 hover:!text-white/85 analyze-toggle-btn shrink-0"
          style={{ fontSize: 11 }}
        >
          重新扫描
        </MoleButton>
      </div>

      {/* hairline 分隔线（对齐 Uninstall：顶部条无背景，直接浮在渐变上） */}
      <div className="shrink-0 h-px bg-white/[0.10]" />
    </>
  )
}
