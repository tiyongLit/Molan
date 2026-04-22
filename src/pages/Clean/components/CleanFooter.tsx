import { ConfigProvider } from 'antd'
import { useI18n } from '@/i18n'
import { MoleButton, MoleCheckbox } from '@/components/ui'

interface CleanFooterProps {
  executionAllowed: boolean
  onResetToDefault: () => void
  /** 直接永久删除（不经过废纸篓），默认 true */
  permanentDelete: boolean
  onPermanentDeleteChange: (value: boolean) => void
}

/**
 * 底部操作区：分隔线 + 左侧删除模式开关 + 右侧恢复默认勾选。
 * 由调用方控制渲染时机（仅 review 阶段）。
 */
export default function CleanFooter({
  executionAllowed,
  onResetToDefault,
  permanentDelete,
  onPermanentDeleteChange,
}: CleanFooterProps) {
  const { t } = useI18n()

  return (
    <div className="pb-[12px] shrink-0">
      <div className="h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-10 w-full" />
      {!executionAllowed && <p role="status" className="text-xs text-white/70 pt-3">{t('clean.executionBlocked')}</p>}
      <div className="flex justify-between items-center pt-3 pb-6 px-1">
        {/* 左侧：删除模式开关 */}
        <label
          className="flex items-center gap-2 cursor-pointer select-none"
          onClick={() => { if (executionAllowed) onPermanentDeleteChange(!permanentDelete) }}
        >
          <MoleCheckbox
            checked={permanentDelete}
            disabled={!executionAllowed}
            onClick={(e) => {
              e.stopPropagation()
              if (executionAllowed) onPermanentDeleteChange(!permanentDelete)
            }}
          />
          <span className="text-xs text-white/50">
            {t('clean.footer.permanentDelete')}
          </span>
        </label>

        {/* 右侧：恢复默认勾选 */}
        <ConfigProvider theme={{
          components: {
            Button: {
              defaultColor: '#dce1e8',
              defaultHoverColor: '#ffffff',
              defaultActiveColor: '#b0bcca',
            }
          }
        }}>
          <MoleButton
            size="small"
            color='default'
            variant='link'
            style={{ fontSize: 12 }}
            onClick={onResetToDefault}>
            {t('clean.footer.resetDefault')}
          </MoleButton>
        </ConfigProvider>
      </div>
    </div>
  )
}
