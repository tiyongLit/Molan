import { useState, useCallback, useEffect, useMemo } from 'react'
import { Segmented, Modal } from 'antd'
import { Type, HardDrive, Clock, ArrowUpDown, ArrowDownUp, ArrowUp, ArrowDown, LayoutGrid, Rocket, Cog, AlertTriangle } from 'lucide-react'
import { UninstallTab, type UninstallSelection, type SortField } from './UninstallTab'
import { UpdatesTab } from './UpdatesTab'
import { StartupTab } from './StartupTab'
import { OrphansTab } from './OrphansTab'
import { UninstallToolbar } from './components/UninstallToolbar'
import { UninstallHistoryList } from './components/UninstallHistoryList'
import useTauri from '@/hooks/useTauri'
import { useUninstallProgress } from '@/hooks/useUninstallProgress'
import { moleMessage } from '@/components/ui'
import { iconService } from '@/utils/iconService'
import { AvatarStack } from '@/components/business/Apps/AvatarStack'
import type { MoleListAppsEntry, StartupFilter } from '@/types/mole'
import './style.scss'

// ── 页面半透明主题变量：深色遮罩，压住青绿亮背景，让白字清晰 ──
const APPS_THEME_VARS: React.CSSProperties = {
  '--bg-page': 'rgba(0, 0, 0, 0.18)',
  '--bg-card': 'rgba(0, 0, 0, 0.28)',
  '--border': 'rgba(255, 255, 255, 0.14)',
} as React.CSSProperties

type AppTab = 'uninstall' | 'updates' | 'startup' | 'orphans'

// ── 启动 tab：筛选下拉（App 分组模式：全部/应用/服务/问题）──
const STARTUP_FILTER_LABEL: Record<StartupFilter, string> = {
  all: '全部',
  apps: '应用',
  services: '服务',
  problems: '问题',
}

const startupFilterMenuItems = [
  { key: 'all', label: <span className="flex items-center gap-2"><LayoutGrid size={13} />全部</span> },
  { key: 'apps', label: <span className="flex items-center gap-2"><Rocket size={13} />应用</span> },
  { key: 'services', label: <span className="flex items-center gap-2"><Cog size={13} />服务</span> },
  { key: 'problems', label: <span className="flex items-center gap-2"><AlertTriangle size={13} />问题</span> },
]

// ── 排序：字段标签 + 各字段默认方向 ──
const SORT_LABEL: Record<SortField, string> = {
  name: '名称',
  size: '大小',
  recent: '最近使用',
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
  const [apps, setApps] = useState<MoleListAppsEntry[]>([])
  const [loading, setLoading] = useState(true)
  const { cleaningApps, startCleaning } = useUninstallProgress()
  const [historyVisible, setHistoryVisible] = useState(false)

  const loadApps = useCallback(() => {
    setLoading(true)
    tauri.mole_list_apps()
      .then(async (list: MoleListAppsEntry[]) => {
        // 先预加载图标，再设置 apps（确保首次渲染即显示原生图标，无闪烁）
        await iconService.preloadIcons((list ?? []).map((a) => a.path))
        setApps(list ?? [])
        // 图标已缓存，无需二次 setApps 触发重渲染
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
      return (
        <>
          已选 <strong className="text-white">{selectedApps.length}</strong>{' '}
          个应用 ·{' '}
          <strong className="text-white font-mono">
            {selectedSize >= 1024 * 1024
              ? `${(selectedSize / 1024 / 1024).toFixed(1)} MB`
              : `${(selectedSize / 1024).toFixed(0)} KB`}
          </strong>
        </>
      )
    }
    
    const { totalApps, completedApps, successCount, errorCount, operationType } = uninstallProgress
    const actionText = operationType === 'uninstall' ? '卸载' : '清理数据'
    
    if (completedApps < totalApps) {
      // 进行中：显示进度
      return (
        <>
          正在{actionText} {totalApps} 个应用...{' '}
          <strong className="text-white">
            ({completedApps}/{totalApps} 完成)
          </strong>
        </>
      )
    } else {
      // 完成：显示结果
      return (
        <>
          {actionText}完成 ✓{' '}
          <strong className="text-emerald-400">
            ({successCount} 成功
            {errorCount > 0 && `, ${errorCount} 失败`})
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

    // 根据模式显示不同的确认弹窗
    const modalConfig = dataOnly
      ? {
          title: `清理 ${targets.length} 个应用的数据？`,
          content: (
            <div>
              <p className="mb-2">
                应用本体将保留，仅清理 <strong>{residualCount}</strong> 个残留文件（缓存、配置、支持文件等）。
              </p>
              <p className="text-xs text-gray-500">
                这些文件将移动到废纸篓（可恢复）。执行时将弹出系统认证框，一次授权后会话内免密。
              </p>
            </div>
          ),
          okText: '清理数据',
        }
      : {
          title: `卸载 ${targets.length} 个应用？`,
          content: '这些应用将移动到废纸篓（可恢复）。执行时将弹出系统认证框，一次授权后会话内免密。',
          okText: '卸载',
        }

    // 快速确认框（非阻塞，对齐"清数据不卸载"交互）
    const confirmed = await new Promise<boolean>((resolve) => {
      Modal.confirm({
        title: modalConfig.title,
        content: modalConfig.content,
        okText: modalConfig.okText,
        okButtonProps: { danger: true },
        cancelText: '取消',
        onOk: () => resolve(true),
        onCancel: () => resolve(false),
      })
    })

    if (!confirmed) return

    // 前置授权（对齐 Clean/Optimize 与 Burrow「入口先弹认证面板」）：
    // 卸载 /Applications 下的 app 及 root 残留需要管理员权限，在 apply 前
    // 弹原生认证面板（幂等，会话内免密），避免 batch 执行中途突然弹窗。
    // 用户取消 → 不开始卸载，静默退出；失败 → toast 提示并退出。
    const auth = (await tauri
      .mole_request_admin_session({
        prompt: dataOnly ? '清理应用数据需要管理员权限' : '卸载应用需要管理员权限',
      })
      .catch(() => ({ authorized: false, status: 'failed' }))) as {
      authorized: boolean
      status: 'authorized' | 'canceled' | 'failed'
    }
    if (!auth.authorized) {
      if (auth.status === 'failed') {
        moleMessage.error('管理员认证失败，无法执行操作')
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
      Modal.error({ title: dataOnly ? '清理数据失败' : '卸载失败', content: String(err) })
    } finally {
      setUninstalling(false)
    }
  }, [selection, apps, uninstalling, tauri, loadApps, startCleaning])

  const handleClearDataOnly = useCallback(
    async (app: MoleListAppsEntry) => {
      if (uninstalling) return

      // 快速确认框（非阻塞）
      const confirmed = await new Promise<boolean>((resolve) => {
        Modal.confirm({
          title: `清理 ${app.display_name || app.name} 的数据？`,
          content: (
            <div>
              <p className="mb-2">
                应用本体将保留，仅清理残留文件（缓存、配置、支持文件等）。
              </p>
              <p className="text-xs text-gray-500">
                这些文件将移动到废纸篓（可恢复）。执行时将弹出系统认证框，一次授权后会话内免密。
              </p>
            </div>
          ),
          okText: '清理数据',
          okButtonProps: { danger: true },
          cancelText: '取消',
          onOk: () => resolve(true),
          onCancel: () => resolve(false),
        })
      })

      if (!confirmed) return

      // 立即显示进度条（不等后端事件）
      startCleaning(app.path, app.display_name || app.name)

      // 前置授权
      const auth = (await tauri
        .mole_request_admin_session({
          prompt: '清理应用数据需要管理员权限',
        })
        .catch(() => ({ authorized: false, status: 'failed' }))) as {
        authorized: boolean
        status: 'authorized' | 'canceled' | 'failed'
      }
      if (!auth.authorized) {
        if (auth.status === 'failed') {
          moleMessage.error('管理员认证失败，无法执行操作')
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
        Modal.error({ title: '清理数据失败', content: String(err) })
      } finally {
        setUninstalling(false)
      }
    },
    [uninstalling, tauri, loadApps, startCleaning]
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
    { key: 'name', label: <span className="flex items-center gap-2"><Type size={13} />名称</span> },
    { key: 'size', label: <span className="flex items-center gap-2"><HardDrive size={13} />大小</span> },
    { key: 'recent', label: <span className="flex items-center gap-2"><Clock size={13} />最近使用</span> },
    { type: 'divider' as const },
    {
      key: 'toggle-direction',
      label: <span className="flex items-center gap-2"><ArrowDownUp size={13} />{sortAscending ? '切换为降序' : '切换为升序'}</span>,
    },
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
            { label: '卸载', value: 'uninstall' },
            { label: '更新', value: 'updates' },
            { label: '启动项', value: 'startup' },
            { label: '残留孤儿', value: 'orphans' },
          ]}
          className="apps-segmented"
        />

        <div className="flex-1" />

        {/* 右侧工具栏：随 tab 变化 */}
        {activeTab === 'uninstall' && (
          <UninstallToolbar
            searchText={searchText}
            onSearchChange={setSearchText}
            searchPlaceholder="搜索应用"
            trigger={
              <>
                <ArrowUpDown size={12} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                <span className="opacity-85 group-hover:opacity-100 transition-opacity">{SORT_LABEL[sortField]}</span>
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
            searchPlaceholder="搜索启动项"
            trigger={
              <>
                <LayoutGrid size={12} className="opacity-60 group-hover:opacity-100 transition-opacity" />
                <span className="opacity-85 group-hover:opacity-100 transition-opacity">{STARTUP_FILTER_LABEL[startupFilter]}</span>
              </>
            }
            menuItems={startupFilterMenuItems}
            selectedKeys={[startupFilter]}
            onMenuClick={(key) => setStartupFilter(key as StartupFilter)}
            onRefresh={handleRescan}
          />
        )}
        {activeTab === 'updates' && (
          <span className="text-[10px] text-white/60">检查更新会访问 Apple 与厂商服务器</span>
        )}
        {activeTab === 'orphans' && (
          // 残留孤儿 tab 的扫描按钮组由 OrphansTab 通过 Portal 注入此槽位（仅扫描完成后出现）
          <div id="orphans-toolbar-slot" className="flex items-center" />
        )}
      </div>

      {/* hairline（原容器 mx-24 + 自身 mr-12 的视觉：左 24 / 右 36） */}
      <div className="shrink-0 h-px bg-gradient-to-r from-transparent via-white/35 to-transparent ml-[24px] mr-[36px]" />

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
                    取消选择
                  </button>
                  <button
                    onClick={handleUninstall}
                    disabled={uninstalling}
                    className="apps-primary-btn text-xs px-4 py-1.5 rounded-md font-bold disabled:opacity-50"
                  >
                    {uninstalling 
                      ? (isDataOnlyMode ? '清理数据中…' : '卸载中…') 
                      : (isDataOnlyMode 
                        ? `清理 ${selectedApps.length} 个应用的数据` 
                        : `卸载 ${selectedApps.length} 个应用`)}
                  </button>
                </div>
              </>
            ) : (
              <span className="text-xs text-white/60">
                勾选要卸载的应用，或展开查看残留文件
              </span>
            )}
          </div>
        </>
      )}
    </div>
  )
}
