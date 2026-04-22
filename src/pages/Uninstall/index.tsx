import { useState, useCallback, useEffect, useMemo } from 'react'
import { Segmented } from 'antd'
import { Type, HardDrive, Clock, ArrowUpDown, ArrowDownUp, ArrowUp, ArrowDown, LayoutGrid, Rocket, Cog, AlertTriangle, Trash2, X } from 'lucide-react'
import { listen } from '@tauri-apps/api/event'
import { UninstallTab, type UninstallSelection, type SortField } from './UninstallTab'
import { UpdatesTab } from './UpdatesTab'
import { StartupTab } from './StartupTab'
import { OrphansTab } from './OrphansTab'
import { UninstallToolbar } from './components/UninstallToolbar'
import { UninstallHistoryList } from './components/UninstallHistoryList'
import useTauri from '@/hooks/useTauri'
import { useUninstallProgress } from '@/hooks/useUninstallProgress'
import { moleMessage } from '@/components/ui'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import { nativeIconRegistry } from '@/utils/nativeIconRegistry'
import { AvatarStack } from '@/components/business/Apps/AvatarStack'
import { useSettings } from '@/pages/Settings/useSettings'
import { EVT_RESIDUAL_DETECTED } from '@/constants/tauri-events'
import type { MoleListAppsEntry, StartupFilter } from '@/types/mole'
import { useI18n } from '@/i18n'
import type { TranslationKey } from '@/i18n'
import './style.scss'

// ── 页面半透明主题变量：深色遮罩，压住青绿亮背景，让白字清晰 ──
const APPS_THEME_VARS: React.CSSProperties = {
  '--bg-page': 'rgba(0, 0, 0, 0.18)',
  '--bg-card': 'rgba(0, 0, 0, 0.28)',
  '--border': 'rgba(255, 255, 255, 0.14)',
} as React.CSSProperties

type AppTab = 'uninstall' | 'updates' | 'startup' | 'orphans'

// ── 启动 tab：筛选下拉（App 分组模式：全部/应用/服务/问题）──
const STARTUP_FILTER_LABEL: Record<StartupFilter, TranslationKey> = {
  all: 'uninstall.startupFilter.all',
  apps: 'uninstall.startupFilter.apps',
  services: 'uninstall.startupFilter.services',
  problems: 'uninstall.startupFilter.problems',
}

// ── 排序：字段标签 key + 各字段默认方向 ──
const SORT_LABEL: Record<SortField, TranslationKey> = {
  name: 'uninstall.sort.name',
  size: 'uninstall.sort.size',
  recent: 'uninstall.sort.recent',
}

const SORT_DEFAULT_ASC: Record<SortField, boolean> = {
  name: true, // 名称升序 A→Z
  size: false, // 大小降序 大→小
  recent: false, // 最近使用降序 近→远
}

/**
 * Apps 页壳：顶部 Segmented（卸载 / 更新 / 启动项）+ 右侧工具栏 + 底部操作栏
 *
 * 布局对齐 Burrow SoftwareView：
 *   ① 顶部：Segmented 胶囊分段（左） + 随 tab 变化的右侧工具栏（排序 chips / 刷新 / 搜索）
 *   ② hairline 分隔线
 *   ③ 内容区（随 tab 切换）
 *   ④ bottomBar（仅卸载 tab）
 */
export function ShellUninstall() {
  const [activeTab, setActiveTab] = useState<AppTab>('uninstall')
  // 卸载 tab 的选中/搜索状态提升到壳层，供底部操作栏与工具栏使用
  const [selection, setSelection] = useState<UninstallSelection>({ checkedPaths: new Set() })
  const [searchText, setSearchText] = useState('')
  const [sortField, setSortField] = useState<SortField>('name')
  const [sortAscending, setSortAscending] = useState(true)

  // 启动 tab 工具栏状态（提升到壳层，对齐卸载 tab 的 searchText/sortField 模式）
  const [startupFilter, setStartupFilter] = useState<StartupFilter>('all')
  const [startupSearch, setStartupSearch] = useState('')
  const [startupReloadTick, setStartupReloadTick] = useState(0)

  const tauri = useTauri()
  const { settings } = useSettings()
  const { t } = useI18n()
  const [apps, setApps] = useState<MoleListAppsEntry[]>([])
  const [loading, setLoading] = useState(true)
  const { cleaningApps, startCleaning } = useUninstallProgress()
  const [historyVisible, setHistoryVisible] = useState(false)

  // ── 卸载残留自动检测 ──
  const [residualApp, setResidualApp] = useState<string | null>(null)

  useEffect(() => {
    if (!settings.uninstall.autoDetectResidual) return
    const unlisten = listen<{ appName: string }>(EVT_RESIDUAL_DETECTED, (event) => {
      setResidualApp(event.payload.appName)
    })
    return () => { unlisten.then(fn => fn()) }
  }, [settings.uninstall.autoDetectResidual])

  const loadApps = useCallback(() => {
    setLoading(true)
    tauri.mole_list_apps()
      .then(async (list: MoleListAppsEntry[]) => {
        // 先预加载图标，再设置 apps（AppIcon 订阅注册表，命中即渲染原生图标，无闪烁）
        await nativeIconRegistry.resolveIdle((list ?? []).map((a) => a.path))
        setApps(list ?? [])
      })
      .catch(() => setApps([]))
      .finally(() => setLoading(false))
  }, [tauri])

  useEffect(() => {
    loadApps()
  }, [loadApps])

  const [uninstalling, setUninstalling] = useState(false)

  // 计算卸载进度（用于底部操作栏动态文案）
  const uninstallProgress = useMemo(() => {
    const totalApps = selection.checkedPaths.size
    const cleaningAppsList = Array.from(cleaningApps.values())
    
    const completedApps = cleaningAppsList.filter(
      app => app.buttonState === 'success' || app.buttonState === 'error'
    ).length
    
    const successCount = cleaningAppsList.filter(
      app => app.buttonState === 'success'
    ).length
    
    const errorCount = cleaningAppsList.filter(
      app => app.buttonState === 'error'
    ).length
    
    const operationType = cleaningAppsList[0]?.operationType || 'uninstall'
    
    return {
      totalApps,
      completedApps,
      successCount,
      errorCount,
      isUninstalling: uninstalling,
      operationType,
      percentage: totalApps > 0 ? Math.round((completedApps / totalApps) * 100) : 0
    }
  }, [selection.checkedPaths, cleaningApps, uninstalling])

  // 动态文案：根据卸载进度显示不同的文案
  const getStatusText = () => {
    if (!uninstallProgress.isUninstalling) {
      // 选择状态：显示选择信息
      const sizeText =
        selectedSize >= 1024 * 1024
          ? `${(selectedSize / 1024 / 1024).toFixed(1)} MB`
          : `${(selectedSize / 1024).toFixed(0)} KB`
      return t('uninstall.status.selected', { count: selectedApps.length, size: sizeText })
    }

    const { totalApps, completedApps, successCount, errorCount, operationType } = uninstallProgress
    const isUninstall = operationType === 'uninstall'

    if (completedApps < totalApps) {
      // 进行中：显示进度
      return (
        <>
          {isUninstall
            ? t('uninstall.status.uninstalling', { count: totalApps })
            : t('uninstall.status.cleaningData', { count: totalApps })}{' '}
          <strong className="text-white">
            {t('uninstall.status.progress', { completed: completedApps, total: totalApps })}
          </strong>
        </>
      )
    } else {
      // 完成：显示结果
      return (
        <>
          {isUninstall ? t('uninstall.status.uninstallDone') : t('uninstall.status.cleanDone')}{' '}
          <strong className="text-emerald-400">
            {errorCount > 0
              ? t('uninstall.status.resultWithError', { success: successCount, failed: errorCount })
              : t('uninstall.status.resultOk', { count: successCount })}
          </strong>
        </>
      )
    }
  }

  const handleUninstall = useCallback(async () => {
    // 只取顶层 app（忽略展开后勾选的残留 path），整 app 卸载。
    const targets = Array.from(selection.checkedPaths)
      .map((p) => apps.find((a) => a.path === p))
      .filter((a): a is MoleListAppsEntry => Boolean(a))
    if (targets.length === 0 || uninstalling) return

    // 检测 Clear Data 模式：如果 app bundle 本身没被勾选，但残留被勾选了
    // 说明用户点了"清数据不卸载"按钮
    const dataOnly = targets.every((a) => !selection.checkedPaths.has(a.path))

    // 统计残留文件数量（用于确认弹窗展示）
    const residualCount = Array.from(selection.checkedPaths).filter(
      (p) => !targets.some((a) => a.path === p)
    ).length

    // 确认弹窗（原生 NSAlert：粗体主文案 + 常规副文案）
    const confirmed = dataOnly
      ? await moleNativeConfirm(t('uninstall.confirm.clearDataTitle', { count: targets.length }), {
          informativeText: t('uninstall.confirm.clearDataBody', { count: residualCount }),
          okLabel: t('uninstall.confirm.okClearData'),
          kind: 'warning',
        })
      : await moleNativeConfirm(t('uninstall.confirm.uninstallTitle', { count: targets.length }), {
          informativeText: t('uninstall.confirm.uninstallBody'),
          okLabel: t('uninstall.confirm.okUninstall'),
          kind: 'warning',
        })
    if (!confirmed) return

    // 前置授权（对齐 Clean/Optimize 与 Burrow「入口先弹认证面板」）：
    // 卸载 /Applications 下的 app 及 root 残留需要管理员权限，在 apply 前
    // 弹原生认证面板（幂等，会话内免密），避免 batch 执行中途突然弹窗。
    // 用户取消 → 不开始卸载，静默退出；失败 → toast 提示并退出。
    const auth = (await tauri
      .mole_request_admin_session({
        prompt: dataOnly ? t('uninstall.authPrompt.clearData') : t('uninstall.authPrompt.uninstall'),
      })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.error(t('uninstall.error.authFailed'))
      }
      return
    }
    
    // 立即关闭确认框，显示内嵌进度（对齐"清数据不卸载"交互）
    // 为每个目标 app 启动清理状态
    targets.forEach((app) => {
      startCleaning(app.path, app.display_name || app.name, dataOnly ? 'clearData' : 'uninstall')
    })
    
    setUninstalling(true)
    
    // 在后台执行卸载（不阻塞 UI）
    try {
      await tauri.mole_uninstall_batch({
        app_paths: targets.map((a) => a.path),
        data_only: dataOnly,
      })
      setSelection({ checkedPaths: new Set() })
      loadApps()
    } catch (err) {
      console.error('[Apps] 卸载失败', err)
      await moleNativeConfirm(
        dataOnly ? t('uninstall.error.clearDataFailed') : t('uninstall.error.uninstallFailed'),
        {
          informativeText: String(err),
          kind: 'error',
          okLabel: t('common.gotIt'),
          cancelLabel: null, // 单按钮提示模式
        }
      )
    } finally {
      setUninstalling(false)
    }
  }, [selection, apps, uninstalling, tauri, loadApps, startCleaning, t])

  const handleClearDataOnly = useCallback(
    async (app: MoleListAppsEntry) => {
      if (uninstalling) return

      // 确认弹窗（原生 NSAlert）
      const confirmed = await moleNativeConfirm(
        t('uninstall.confirm.clearDataAppTitle', { name: app.display_name || app.name }),
        {
          informativeText: t('uninstall.confirm.clearDataAppBody'),
          okLabel: t('uninstall.confirm.okClearData'),
          kind: 'warning',
        }
      )

      if (!confirmed) return

      // 立即显示进度条（不等后端事件）
      startCleaning(app.path, app.display_name || app.name)

      // 前置授权
      const auth = (await tauri
        .mole_request_admin_session({
          prompt: t('uninstall.authPrompt.clearData'),
        })
        .catch(() => ({ authorized: false, status: 'failed' }))) as {
        authorized: boolean
        status: 'authorized' | 'canceled' | 'failed'
      }
      if (!auth.authorized) {
        if (auth.status === 'failed') {
          moleMessage.error(t('uninstall.error.authFailed'))
        }
        return
      }

      // 启动后台清理（不阻塞 UI）
      setUninstalling(true)
      try {
        await tauri.mole_uninstall_batch({
          app_paths: [app.path],
          data_only: true,
        })
        // Clear Data 模式不刷新列表（app 本体保留），结果通过事件显示在按钮上
        setSelection({ checkedPaths: new Set() })
      } catch (err) {
        console.error('[Apps] 清理数据失败', err)
        await moleNativeConfirm(t('uninstall.error.clearDataFailed'), {
          informativeText: String(err),
          kind: 'error',
          okLabel: t('common.gotIt'),
          cancelLabel: null, // 单按钮提示模式
        })
      } finally {
        setUninstalling(false)
      }
    },
    [uninstalling, tauri, loadApps, startCleaning, t]
  )

  const handleSelectionChange = useCallback((s: UninstallSelection) => {
    setSelection(s)
  }, [])

  const handleRescan = useCallback(() => {
    if (activeTab === 'uninstall') {
      setSearchText('')
      loadApps()
    } else if (activeTab === 'startup') {
      // 信号驱动 StartupTab 重扫（已扫描过登录项则保持含 BTM）
      setStartupReloadTick((t) => t + 1)
    }
  }, [activeTab, loadApps])

  const handleSortSelect = useCallback(
    (key: string) => {
      if (key === 'toggle-direction') {
        setSortAscending((v) => !v)
        return
      }
      const field = key as SortField
      if (field !== sortField) {
        setSortField(field)
        setSortAscending(SORT_DEFAULT_ASC[field])
      }
    },
    [sortField]
  )

  const sortMenuItems = [
    { key: 'name', label: <span className="flex items-center gap-2"><Type size={13} />{t('uninstall.sort.name')}</span> },
    { key: 'size', label: <span className="flex items-center gap-2"><HardDrive size={13} />{t('uninstall.sort.size')}</span> },
    { key: 'recent', label: <span className="flex items-center gap-2"><Clock size={13} />{t('uninstall.sort.recent')}</span> },
    { type: 'divider' as const },
    {
      key: 'toggle-direction',
      label: <span className="flex items-center gap-2"><ArrowDownUp size={13} />{sortAscending ? t('uninstall.sort.toDesc') : t('uninstall.sort.toAsc')}</span>,
    },
  ]

  const startupFilterMenuItems = [
    { key: 'all', label: <span className="flex items-center gap-2"><LayoutGrid size={13} />{t('uninstall.startupFilter.all')}</span> },
    { key: 'apps', label: <span className="flex items-center gap-2"><Rocket size={13} />{t('uninstall.startupFilter.apps')}</span> },
    { key: 'services', label: <span className="flex items-center gap-2"><Cog size={13} />{t('uninstall.startupFilter.services')}</span> },
    { key: 'problems', label: <span className="flex items-center gap-2"><AlertTriangle size={13} />{t('uninstall.startupFilter.problems')}</span> },
  ]

  const selectedApps = Array.from(selection.checkedPaths)
    .map((p) => apps.find((a) => a.path === p))
    .filter((a): a is MoleListAppsEntry => Boolean(a))

  const selectedSize = selectedApps.reduce((s, a) => s + (a.size_bytes || 0), 0)

  // 检测 Clear Data 模式：如果所有选中的 app 的 bundle 都没被勾选，但残留被勾选了
  const isDataOnlyMode = selectedApps.length > 0 && 
    selectedApps.every((a) => !selection.checkedPaths.has(a.path))

  return (
    // 滚动条贴边策略（方案 A）：根容器不带水平 margin，SimpleBar 右缘贴窗口右缘；
    // 顶栏/hairline/底栏自带 mx-24 留白，列表缩进由各 Tab 滚动区 wrapper 补偿（px-24）。
    <div className="h-full flex flex-col" style={APPS_THEME_VARS}>
      {/* ═══ 顶部：Segmented + 右侧工具栏 ═══ */}
      <div className="flex items-center gap-3 h-[48px] shrink-0 mr-[24px]">
        <Segmented
          value={activeTab}
          onChange={(v) => setActiveTab(v as AppTab)}
          options={[
            { label: t('uninstall.tab.uninstall'), value: 'uninstall' },
            { label: t('uninstall.tab.updates'), value: 'updates' },
            { label: t('uninstall.tab.startup'), value: 'startup' },
            { label: t('uninstall.tab.orphans'), value: 'orphans' },
          ]}
          className="apps-segmented"
        />

        <div className="flex-1" />

        {/* 右侧工具栏：随 tab 变化 */}
        {activeTab === 'uninstall' && (
          <UninstallToolbar
            searchText={searchText}
            onSearchChange={setSearchText}
            searchPlaceholder={t('uninstall.search.apps')}
            trigger={
              <>
                <ArrowUpDown size={12} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                <span className="opacity-85 group-hover:opacity-100 transition-opacity">{t(SORT_LABEL[sortField])}</span>
                {sortAscending ? (
                  <ArrowUp size={10} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                ) : (
                  <ArrowDown size={10} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                )}
              </>
            }
            menuItems={sortMenuItems}
            selectedKeys={[sortField]}
            onMenuClick={handleSortSelect}
            onRefresh={handleRescan}
            onShowHistory={() => setHistoryVisible(!historyVisible)}
            showHistory={historyVisible}
          />
        )}
        {activeTab === 'startup' && (
          <UninstallToolbar
            searchText={startupSearch}
            onSearchChange={setStartupSearch}
            searchPlaceholder={t('uninstall.search.startup')}
            trigger={
              <>
                <LayoutGrid size={12} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                <span className="opacity-85 group-hover:opacity-100 transition-opacity">{t(STARTUP_FILTER_LABEL[startupFilter])}</span>
              </>
            }
            menuItems={startupFilterMenuItems}
            selectedKeys={[startupFilter]}
            onMenuClick={(key) => setStartupFilter(key as StartupFilter)}
            onRefresh={handleRescan}
          />
        )}
        {activeTab === 'updates' && (
          <span className="text-[10px] text-white/60">{t('uninstall.updates.toolbarHint')}</span>
        )}
        {activeTab === 'orphans' && (
          // 残留孤儿 tab 的扫描按钮组由 OrphansTab 通过 Portal 注入此槽位（仅扫描完成后出现）
          <div id="orphans-toolbar-slot" className="flex items-center" />
        )}
      </div>

      {/* hairline（原容器 mx-24 + 自身 mr-12 的视觉：左 24 / 右 36） */}
      <div className="shrink-0 h-px bg-gradient-to-r from-transparent via-white/35 to-transparent ml-[24px] mr-[36px]" />

      {/* ═══ 卸载残留检测通知条 ═══ */}
      {residualApp && (
        <div className="shrink-0 flex items-center gap-2 mx-[24px] mt-2 rounded-lg bg-amber-500/10 border border-amber-500/20 px-3 py-2">
          <Trash2 size={14} className="shrink-0 text-amber-400" />
          <span className="text-[11px] text-amber-200/90 flex-1 min-w-0 truncate">
            {t('uninstall.residual.detected', { app: residualApp })}
          </span>
          <button
            onClick={() => { setActiveTab('orphans'); setResidualApp(null) }}
            className="shrink-0 rounded px-2 py-0.5 text-[10px] font-medium text-amber-300 bg-amber-500/15 hover:bg-amber-500/25 transition-colors"
          >
            {t('uninstall.residual.scan')}
          </button>
          <button
            onClick={() => setResidualApp(null)}
            className="shrink-0 p-0.5 rounded text-amber-400/50 hover:text-amber-300 hover:bg-amber-500/10 transition-colors"
          >
            <X size={12} />
          </button>
        </div>
      )}

      {/* ═══ 内容区 ═══ */}
      <div className="flex-1 min-h-0 overflow-hidden">
        {activeTab === 'uninstall' && (
          historyVisible ? (
            <UninstallHistoryList />
          ) : (
            <UninstallTab
              apps={apps}
              loading={loading}
              searchText={searchText}
              selection={selection}
              sortField={sortField}
              sortAscending={sortAscending}
              onSelectionChange={handleSelectionChange}
              onClearDataOnly={handleClearDataOnly}
              cleaningApps={cleaningApps}
            />
          )
        )}
        {activeTab === 'updates' && <UpdatesTab apps={apps} />}
        {activeTab === 'startup' && (
          <StartupTab
            filter={startupFilter}
            searchText={startupSearch}
            reloadTick={startupReloadTick}
          />
        )}
        {activeTab === 'orphans' && <OrphansTab />}
      </div>

      {/* ═══ 底部操作栏（仅卸载 tab）═══ */}
      {activeTab === 'uninstall' && (
        <>
          <div className="shrink-0 h-px bg-gradient-to-r from-transparent via-white/35 to-transparent mx-[24px]" />
          <div className="shrink-0 flex items-center justify-between mx-[24px] h-[44px]">
            {selectedApps.length > 0 ? (
              <>
                <div className="flex items-center gap-2 min-w-0">
                  {/* 选中应用图标堆栈（重叠头像 + hover 展开 + 徽标） */}
                  <AvatarStack 
                    apps={selectedApps} 
                    maxDisplay={3} 
                    iconSize={24} 
                  />
                  <span className="text-xs text-white/85 truncate">
                    {getStatusText()}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleSelectionChange({ checkedPaths: new Set() })}
                    className="apps-ghost-btn text-xs px-3 py-1.5 rounded-md transition-colors"
                  >
                    {t('uninstall.action.deselect')}
                  </button>
                  <button
                    onClick={handleUninstall}
                    disabled={uninstalling}
                    className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold disabled:opacity-50"
                  >
                    {uninstalling
                      ? (isDataOnlyMode ? t('uninstall.action.cleaningDataNow') : t('uninstall.action.uninstallingNow'))
                      : (isDataOnlyMode
                        ? t('uninstall.action.clearDataFor', { count: selectedApps.length })
                        : t('uninstall.action.uninstallApps', { count: selectedApps.length }))}
                  </button>
                </div>
              </>
            ) : (
              <span className="text-xs text-white/60">
                {t('uninstall.action.emptyHint')}
              </span>
            )}
          </div>
        </>
      )}
    </div>
  )
}
