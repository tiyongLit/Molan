import { Switch, Divider } from 'antd'
import type { AppSettings } from '../useSettings'
import { MolePopSelect } from '@/components/ui'
import { useI18n, LOCALE_LABELS, SUPPORTED_LOCALES } from '@/i18n'
import type { AppLocale } from '@/i18n'

interface Props {
  settings: AppSettings
  trashSaving: boolean
  trashError: string
  onToggleAutoLaunch: (enable: boolean) => void
  onChangeLanguage: (locale: AppLocale) => void
  onUpdateSetting: <K extends keyof AppSettings>(
    key: K,
    value: AppSettings[K] | ((prev: AppSettings[K]) => AppSettings[K])
  ) => void
}

/**
 * 偏好设置（扁平布局，样式对齐柠檬 PreferenceViewController）：
 * - 标题 14px 白（柠檬 systemFontOfSize:14 + getTitleColor），开关与标题垂直居中
 * - 描述 12px #94979B（柠檬 getLightSystemFont:12），在标题下方 8px、通栏展示
 * - 子项标签 12px 白（柠檬 createLabelForItem），与标题同一左边距、距上方 12px
 * - Divider 只在块与块之间（14px 上下），父子项之间不加
 */
export function GeneralSection({ settings, trashSaving, trashError, onToggleAutoLaunch, onChangeLanguage, onUpdateSetting }: Props) {
  const { t } = useI18n()
  return (
    <div className="flex flex-col">
      {/* ── 块 0：语言 ── */}
      <SettingRow
        title={t('settings.language.title')}
        description={t('settings.language.description')}
        control={
          <MolePopSelect<AppLocale>
            value={settings.language}
            onChange={onChangeLanguage}
            minWidth={112}
            options={SUPPORTED_LOCALES.map((loc) => ({ value: loc, label: LOCALE_LABELS[loc] }))}
          />
        }
      />
      <Divider style={{ margin: '14px 0' }} />

      {/* ── 块 1：开机时启动 ── */}
      <SettingRow
        title={t('settings.autoLaunch.title')}
        description={t('settings.autoLaunch.description')}
        control={
          <Switch size="small" checked={settings.autoLaunch} onChange={onToggleAutoLaunch} />
        }
      />
      <Divider style={{ margin: '14px 0' }} />

      {/* ── 块 2：自动检查更新 + 检查频率（同一视觉块） ── */}
      <SettingRow
        title={t('settings.autoUpdate.title')}
        description={t('settings.autoUpdate.description')}
        control={
          <Switch
            size="small"
            checked={settings.autoCheckUpdate}
            onChange={(checked) => onUpdateSetting('autoCheckUpdate', checked)}
          />
        }
      />
      {settings.autoCheckUpdate && (
        <SettingSubRow
          title={t('settings.updateInterval')}
          control={
            <MolePopSelect
              value={settings.updateCheckInterval}
              onChange={(val) => onUpdateSetting('updateCheckInterval', val)}
              minWidth={84}
              options={[
                { value: 1, label: t('settings.updateInterval.daily') },
                { value: 7, label: t('settings.updateInterval.weekly') },
                { value: 30, label: t('settings.updateInterval.monthly') },
              ]}
            />
          }
        />
      )}
      <Divider style={{ margin: '14px 0' }} />

      {/* ── 块 3：废纸篓清理提醒 + 提醒阈值（同一视觉块） ── */}
      <SettingRow
        title={t('trashReminder.settingsTitle')}
        description={t('trashReminder.settingsDescription')}
        control={
          <Switch
            size="small"
            disabled={trashSaving}
            loading={trashSaving}
            checked={settings.trashReminder.enabled}
            onChange={(checked) => onUpdateSetting('trashReminder', (prev) => ({ ...prev, enabled: checked }))}
          />
        }
      />
      {trashError && <p role="alert" className="mt-2 text-[12px] text-[#f87171]">{trashError}</p>}
      {settings.trashReminder.enabled && (
        <SettingSubRow
          title={t('trashReminder.threshold')}
          control={
            <MolePopSelect
              value={settings.trashReminder.threshold}
              onChange={(val) => onUpdateSetting('trashReminder', (prev) => ({ ...prev, threshold: val }))}
              disabled={trashSaving}
              minWidth={84}
              options={[
                { value: 1, label: '1 MB' },
                { value: 10, label: '10 MB' },
                { value: 50, label: '50 MB' },
                { value: 512, label: '512 MB' },
                { value: 1024, label: '1 GB' },
                { value: 2048, label: '2 GB' },
              ]}
            />
          }
        />
      )}
      <Divider style={{ margin: '14px 0' }} />

      {/* ── 块 4：内存列表关闭按钮 ── */}
      <SettingRow
        title={t('settings.killProcess.title')}
        description={t('settings.killProcess.description')}
        control={
          <Switch
            size="small"
            checked={settings.dashboard.enableKillProcess}
            onChange={(checked) => onUpdateSetting('dashboard', (prev) => ({ ...prev, enableKillProcess: checked }))}
          />
        }
      />
      <Divider style={{ margin: '14px 0' }} />

      {/* ── 块 5：自动检测卸载残留（原 UninstallSection 合并至此） ── */}
      <SettingRow
        title={t('settings.residual.title')}
        description={t('settings.residual.description')}
        control={
          <Switch
            size="small"
            checked={settings.uninstall.autoDetectResidual}
            onChange={(checked) =>
              onUpdateSetting('uninstall', (prev) => ({ ...prev, autoDetectResidual: checked }))
            }
          />
        }
      />
    </div>
  )
}

// ── 复用组件（布局结构对齐柠檬 PreferenceViewController） ──

/** 主设置行：标题 14px 白 + 右侧开关（与标题垂直居中）；描述 12px 蓝灰在标题下方通栏 */
function SettingRow({
  title,
  description,
  control,
}: {
  title: string
  description?: string
  control: React.ReactNode
}) {
  return (
    <div>
      <div className="flex items-center justify-between">
        <span className="text-[14px] font-medium text-white leading-tight min-w-0">{title}</span>
        <div className="shrink-0 ml-4">{control}</div>
      </div>
      {description && (
        // 描述色使用蓝灰 #B0BCCC 而非中性灰 #94979B，与页面 #73a6e0→#354070 渐变同色相，
        // 在深色区对比度 ~8.5:1，避免中性灰在蓝底上显灰、没精神。
        <p className="mt-2 text-[12px] text-[#B0BCCC] leading-snug">{description}</p>
      )}
    </div>
  )
}

/** 子设置行：标签 12px 白（柠檬 createLabelForItem 同色系），与主标题同一左边距、不缩进 */
function SettingSubRow({ title, control }: { title: string; control: React.ReactNode }) {
  return (
    <div className="mt-3 flex items-center justify-between">
      <span className="text-[12px] text-white leading-tight">{title}</span>
      <div className="shrink-0 ml-4">{control}</div>
    </div>
  )
}
