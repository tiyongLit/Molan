export interface MoleAnalyzeEntry {
  name: string
  path: string
  size: number
  is_dir: boolean
  /** Rust 侧 `skip_serializing_if = is_false`：false 时字段缺失，消费侧用 `?? false` 归一化 */
  insight?: boolean
  cleanable?: boolean
  protected?: boolean
  /** symlink 标记（GUI 扩展：Go CLI 靠 name 的 " →" 后缀，GUI 用显式布尔） */
  is_symlink?: boolean
  /** 目录直接子项统计（GUI 扩展，对标 lemon-cleaner 的 "X items"；零值时字段缺失） */
  child_files?: number
  child_dirs?: number
  child_links?: number
  /** bundle 叶子捷径标记（GUI 扩展：首扫经 Spotlight 聚合大小叶子化，钻取按需子树扫描） */
  is_bundle_leaf?: boolean
  bundle_id?: string
  bundle_display_name?: string
  last_access?: string | null
}

export interface MoleAnalyzeFile {
  name: string
  path: string
  size: number
  last_access?: string | null
}

export interface MoleAnalyzeResult {
  path: string
  overview: boolean
  entries: MoleAnalyzeEntry[]
  large_files: MoleAnalyzeFile[]
  total_size: number
  total_files: number
  diskFree?: number
}

export interface HardwareInfo {
  model: string
  cpu_model: string
  total_ram: string
  disk_size: string
  os_version: string
  refresh_rate: string
}

export interface CpuStatus {
  usage: number
  per_core: number[]
  per_core_estimated: boolean
  core_count: number
  logical_cpu: number
  p_core_count: number
  e_core_count: number
  /** CPU 时间片分解（活动监视器同款，单位 %）。首次采集无前值时为 undefined */
  user_pct?: number
  system_pct?: number
}

export interface MemoryStatus {
  used: number
  total: number
  available: number
  used_percent: number
  swap_used: number
  swap_total: number
  cached: number
  pressure: string
}

export interface DiskStatus {
  mount: string
  device: string
  used: number
  total: number
  /**
   * 可用空间（字节）——后端下发的单一事实来源（Finder/APFS 校准，与「关于本机」同口径）。
   * 旧快照可能缺失该字段，消费方统一经 diskFreeBytes() 兜底，禁止各处自行 total - used。
   */
  free?: number
  used_percent: number
  fstype: string
  external: boolean
}

export interface DiskIoStatus {
  read_rate: number
  write_rate: number
}

export interface NetworkStatus {
  name: string
  rx_rate_mbs: number
  tx_rate_mbs: number
  ip: string
}

export interface NetworkHistory {
  /** 后端 skip_serializing，前端不再收到此字段（保留兼容） */
  rx_history?: number[]
  /** 后端 skip_serializing，前端不再收到此字段（保留兼容） */
  tx_history?: number[]
  /** 最近一次采样下行速率（MB/s） */
  rx_latest: number
  /** 最近一次采样上行速率（MB/s） */
  tx_latest: number
}

export interface ProxyStatus {
  /** Rust 端 serde rename = "type"，JSON key 是 "type" */
  type: string
  host: string
  enabled: boolean
}

export interface ThermalStatus {
  cpu_temp: number
  fan_speed: number
  fan_count: number
}

export interface SensorReading {
  label: string
  value: number
  unit: string
  note: string
}

export interface ProcessInfo {
  pid: number
  ppid: number
  name: string
  command: string
  cpu: number
  memory: number
  /** RSS 物理内存 (bytes), 对齐 Go memory_bytes */
  memory_bytes?: number
  /** 原生应用图标 PNG base64（不含 data: 前缀）：后端按 pid/ppid/name 匹配
   * NSWorkspace runningApplications 索引得到 bundle 路径后，走文件域同一管线
   * （mtime 校验长期缓存）取得；daemon/helper 无 bundle 则缺省，前端显示骨架屏。
   * 仅 top_processes 携带 */
  icon?: string
  /** 应用 bundle 路径（.app）：前端 iconService 的取图键，与 Uninstall 列表的 path
   * 同源，可直接命中文件域磁盘图标缓存（跨窗口共享：托盘与主窗口各自一份内存 LRU，
   * 但磁盘缓存是同一份）。
   * 与 icon 走同一编码器，data URI 逐字节相同，两源先后到位不会产生图标跳变。
   * 仅 top_processes 携带 */
  bundle_path?: string
}

export interface ProcessWatchConfig {
  enabled: boolean
  cpu_threshold: number
  window: string
}

export interface ProcessAlert {
  pid: number
  name: string
  command?: string
  cpu: number
  /** CPU 阈值 */
  threshold: number
  /** 持续窗口 (如 "5m0s") */
  window: string
  /** 告警触发时间 (ISO8601) */
  triggered_at: string
  status: string
}

export interface MoleStatusResult {
  collected_at: string
  host: string
  platform: string
  procs: number
  hardware: HardwareInfo
  health_score: number
  health_score_msg: string
  cpu: CpuStatus
  memory: MemoryStatus
  disks: DiskStatus[]
  trash_size: number
  trash_approx: boolean
  disk_io: DiskIoStatus
  network: NetworkStatus[]
  network_history: NetworkHistory
  proxy: ProxyStatus
  thermal: ThermalStatus
  sensors: SensorReading[] | null
  top_processes: ProcessInfo[]
  process_watch: ProcessWatchConfig
  process_alerts: ProcessAlert[]
}

// ============================================================
// Mole Clean types (cmd/clean/json.go)
// ============================================================

export interface MoleCleanResult {
  mode: string
  collected_at: string
  /** 扫描唯一标识。前端 clean_apply 时须回传此 ID，后端据此验证快照。仅 dry_run 模式返回。 */
  scan_id?: string
  /** 统计口径：`logical`（metadata.len）/ `physical`（blocks*512）。后端回填，前端展示需一致。 */
  size_metric?: string
  whitelist?: {
    active_patterns: number
    /** 与 DEFAULT_WHITELIST_PATTERNS 命中条数（CLI "core"） */
    core_pattern_count: number
    /** 自定义条目数（CLI "custom"） */
    custom_pattern_count: number
    /** 仅 dry_run 返回列表；execute 时省略或为空 */
    patterns?: string[]
    /** 白名单加载时的验证警告（对齐 clean.sh:1098-1103） */
    warnings?: string[]
  }
  categories?: MoleCleanCategory[]
  results?: MoleCleanExecuteResult[]
  summary: MoleCleanSummary
}

export interface MoleCleanCategory {
  id: string
  title: string
  tips: string
  recommend: boolean
  cautious: boolean
  requires_sudo: boolean
  whitelist_matched: boolean
  items: MoleCleanItem[]
}

export interface MoleCleanItem {
  id: string
  path: string
  size: number
  size_human: string
  file_count: number
  status: string
  whitelist_matched: boolean
  /** 后端权威计算的默认勾选状态。前端据此初始化 selectedItemIds，用户可自由修改。 */
  default_selected: boolean
  recommend: boolean
  cautious: boolean
  categoryId: string
  categoryTitle: string
  /** 真实文件系统路径；非空时前端渲染「在 Finder 中显示」按钮（对齐 Lemon）。 */
  real_path?: string
}

export interface MoleCleanExecuteResult {
  category_id: string
  item_id: string
  path: string
  size_cleaned: number
  size_cleaned_human: string
  file_count: number
  status: string
  error?: string | null
}

export interface MoleCleanSummary {
  total_cleanable_size?: number
  total_cleanable_size_human?: string
  total_cleaned_size?: number
  total_cleaned_size_human?: string
  total_file_count: number
  category_count?: number
  success_count?: number
  skipped_count?: number
  failed_count?: number
  final_free_space?: number
  final_free_space_human?: string
  free_space_change?: number
  free_space_change_human?: string
  sudo_required?: boolean
  sudo_items_count?: number
  duration_seconds?: number
}

// ============================================================
// Clean v2 — clean_status / clean_scan / clean_apply 契约
// ============================================================

/** clean_status 返回：进入页面时的权限与时效状态。 */
export interface CleanStatusInfo {
  /** 当前是否拥有管理员会话（影响 system_caches / apple_silicon 等需 sudo 的分类） */
  sudo_session_active: boolean
  /** 最近一次扫描完成时间（ISO8601）；进程重启后为 undefined */
  last_scan_at?: string
}

/** clean_apply 入参：选中项的 id 数组 + 扫描快照标识。 */
export interface CleanApplyArgs {
  /** 格式 "categoryId::itemId" */
  item_ids: string[]
  /** 扫描唯一标识，必须与最近一次 clean_scan 返回的 scan_id 一致 */
  scan_id: string
}

/** clean_apply 单条结果。 */
export interface CleanApplyResultItem {
  category_id: string
  item_id: string
  path: string
  size_cleaned: number
  status: string
  error?: string | null
}

/** clean_apply 返回。 */
export interface CleanApplyResult {
  results: CleanApplyResultItem[]
  summary: {
    total_cleaned_size: number
    success_count: number
    failed_count: number
    free_space_change?: number
    free_space_change_human?: string
  }
}

/** clean::apply-progress 事件 payload（与 Rust CleanApplyProgressPayload 一致，camelCase）。 */
export interface CleanApplyProgressEvent {
  /** "start" | "category_start" | "category_done" | "complete" */
  phase: string
  currentCategory?: string
  currentPath?: string
  cleanedBytes: number
  doneCategories: number
  totalCategories: number
  failedCount: number
}

// ============================================================
// Mole Purge types (cmd/purge/json.go)
// ============================================================

export interface MolePurgeResult {
  mode: string
  collected_at: string
  search_paths: string[]
  projects?: MolePurgeProject[]
  results?: MolePurgeExecuteResult[]
  summary: MolePurgeSummary
}

export interface MolePurgeProject {
  id: string
  name: string
  path: string
  type: string
  indicators: string[]
  artifacts: MolePurgeArtifact[]
  total_artifact_size: number
  total_artifact_size_human: string
}

export interface MolePurgeArtifact {
  name: string
  path: string
  size: number
  size_human: string
  age_days: number
  status: string
}

export interface MolePurgeExecuteResult {
  project_id: string
  project_name: string
  artifact_name: string
  path: string
  size_cleaned: number
  size_cleaned_human: string
  status: string
  error?: string | null
}

export interface MolePurgeSummary {
  total_projects?: number
  total_artifact_size?: number
  total_artifact_size_human?: string
  total_artifact_count: number
  total_cleaned_size?: number
  total_cleaned_size_human?: string
  success_count?: number
  skipped_count?: number
  failed_count?: number
  duration_seconds?: number
}

// ============================================================
// Mole Check types (cmd/check/json.go)
// ============================================================

export interface MoleCheckResult {
  collected_at: string
  checks: MoleCheckItem[]
  summary: MoleCheckSummary
}

export interface MoleCheckItem {
  id: string
  title: string
  status: 'ok' | 'warning' | 'error'
  message: string
  details?: any
}

export interface MoleCheckSummary {
  total_checks: number
  ok_count: number
  warning_count: number
  error_count: number
  auto_fix_available: boolean
  auto_fix_items: string[]
}

// ============================================================
// Mole Optimize types (cmd/optimize/json.go)
// ============================================================

export interface MoleOptimizeResult {
  mode: string
  collected_at: string
  system_info?: MoleOptimizeSystemInfo
  tasks?: MoleOptimizeTask[]
  results?: MoleOptimizeExecuteResult[]
  diagnostics?: MoleOptimizeDiagnostics
  stats?: MoleOptimizeStats
  summary: MoleOptimizeSummary
}

export interface MoleOptimizeDiagnostics {
  has_bottleneck: boolean
  sustained: MoleDiagSustained[]
  detach_candidates: MoleDiagDetachCandidate[]
  primary?: MoleDiagPrimary
  spctl_status?: string
}

export interface MoleDiagSustained {
  family: string
  label: string
  avg_cpu: number
}

export interface MoleDiagDetachCandidate {
  image: string
  mount: string
}

export interface MoleDiagPrimary {
  family: string
  label: string
  avg_cpu: number
  note?: string
}

export interface MoleOptimizeStats {
  cache_cleaned_kb: number
  databases_optimized: number
  configs_repaired: number
}

export interface MoleOptimizeSystemInfo {
  memory_used_gb: number
  memory_total_gb: number
  memory_used_percent: number
  disk_used_gb: number
  disk_total_gb: number
  disk_used_percent: number
  uptime_days: number
}

export interface MoleOptimizeTask {
  id: string
  name: string
  description: string
  safe: boolean
  status: string
  estimated_savings?: string | null
}

export interface MoleOptimizeExecuteResult {
  task_id: string
  task_name: string
  status: string
  space_saved?: number
  space_saved_human?: string
  duration_seconds?: number
  error?: string | null
}

export interface MoleOptimizeSummary {
  total_tasks: number
  safe_count?: number
  applied_count?: number
  would_apply_count?: number
  skipped_count?: number
  failed_count?: number
  /** 六态统计：status → 数量（Rust 控制器 complete 阶段返回） */
  outcomes?: Partial<Record<OptimizeOutcomeStatus, number>>
  estimated_space_savings?: string
  actual_space_savings?: string
  duration_seconds?: number
}

/** 六态结局（与 Rust `OptimizeOutcome` 的 snake_case 序列化一致） */
export type OptimizeOutcomeStatus =
  | 'applied'
  | 'unchanged'
  | 'skipped'
  | 'unavailable'
  | 'attention'
  | 'failed'

/** Optimize 执行阶段流式进度事件（与 Rust `optimize::progress` payload 一致） */
export type OptimizeProgressEvent =
  | { phase: 'begin'; total: number; actions: string[] }
  | {
      phase: 'task_start'
      index: number
      total: number
      action: string
      name: string
      description: string
    }
  | {
      phase: 'task_skipped'
      index: number
      total: number
      action: string
      name: string
      reason: string
    }
  | {
      phase: 'task_done'
      index: number
      total: number
      action: string
      name: string
      ok: boolean
      /** 六态结局（applied/unchanged/skipped/unavailable/attention/failed） */
      outcome: OptimizeOutcomeStatus
      duration_ms: number
      error?: string | null
    }
  | {
      phase: 'complete'
      total: number
      success: number
      failed: number
      skipped: number
      /** 六态统计：status → 数量 */
      outcomes: Partial<Record<OptimizeOutcomeStatus, number>>
      duration_ms: number
    }

// ============================================================
// Mole Uninstall types (cmd/uninstall/json.go)
// ============================================================

export interface MoleUninstallResult {
  mode: string
  data_only?: boolean
  collected_at: string
  app: MoleUninstallAppInfo
  related_files?: MoleUninstallRelatedFile[]
  /** New CLI: system-level files shown for review only (NOT deletable). */
  review_only_files?: MoleUninstallRelatedFile[]
  results?: MoleUninstallExecuteResult[]
  /** New CLI: scan rejected this app (privileged path below a mutable parent / identity unavailable). */
  manual_removal?: boolean
  reason?: string
  summary: MoleUninstallSummary
}

export interface MoleListAppsEntry {
  name: string
  display_name: string
  path: string
  bundle_id: string
  source: string
  uninstall_name: string
  size_bytes: number
  size_human: string
  last_used_epoch: number
  last_used_relative: string
  version: string
  running: boolean
  /** 更新机制来源（对齐 Burrow UpdateSources.detect）："sparkle" | "app_store" | "electron" | null */
  update_source: 'sparkle' | 'app_store' | 'electron' | null
}

export interface MoleUninstallAppInfo {
  name: string
  bundle_id: string
  path: string
  version: string
  size?: number
  size_human?: string
  last_used_epoch?: number
  last_used_relative?: string
  is_brew_cask: boolean
  brew_cask_name?: string | null
  /** New CLI: app requires an official uninstaller. */
  is_official_uninstaller?: boolean
  official_vendor?: string
}

export interface MoleUninstallRelatedFile {
  path: string
  size: number
  size_human: string
  type: string
  has_sensitive_data: boolean
  /** New CLI: true if this is a system-level file shown for review only. */
  review_only?: boolean
}

export interface MoleUninstallExecuteResult {
  path: string
  type: string
  size_cleaned: number
  size_cleaned_human: string
  status: string
  error?: string | null
}

export interface MoleUninstallSummary {
  total_size?: number
  total_size_human?: string
  total_cleaned_size?: number
  total_cleaned_size_human?: string
  file_count: number
  has_sensitive_data: boolean
  sensitive_paths?: string[]
  launch_agents?: string[]
  launch_daemons?: string[]
  success_count?: number
  skipped_count?: number
  failed_count?: number
  duration_seconds?: number
  /** New CLI: apps blocked because they require an official uninstaller. */
  blocked_apps?: string[]
  /** New CLI: apps with Background Items leftover entries detected. */
  background_item_leftovers?: string[]
  /** New CLI: apps that were running but successfully uninstalled. */
  running_at_uninstall_apps?: string[]
}

// ============================================================
// Orphan types (Rust lib/uninstall/orphan_safety.rs，对齐 PureMac OrphanSafetyPolicy)
// ============================================================

/** 孤儿残留分类 */
export type OrphanCategory =
  | 'cache'
  | 'log'
  | 'saved_state'
  | 'http_storage'
  | 'web_kit'
  | 'crash_reporter'
  | 'preference'
  | 'container'
  | 'launch_agent'
  | 'application_support'
  | 'other'

/** 单个孤儿残留条目 */
export interface OrphanEntry {
  /** 完整路径 */
  path: string
  /** 文件名（最后一段） */
  file_name: string
  /** 大小（字节） */
  size_bytes: number
  /** 人类可读大小 */
  size_human: string
  /** 分类 */
  category: OrphanCategory
  /** 是否可删除（白名单内 = true，仅展示 = false） */
  deletable: boolean
}

/** 孤儿删除结果 */
export interface OrphanDeleteResult {
  success_count: number
  failed_count: number
  total_freed_bytes: number
}

// ============================================================
// Updates types (Rust controllers/updates.rs，对齐 Burrow Updates)
// ============================================================

/** brew outdated --json=v2 行条目（对齐 Burrow OutdatedItem） */
export interface BrewOutdatedItem {
  name: string
  installed: string
  latest: string
  /** "formula" | "cask" */
  kind: string
}

/** mole_updates_check 单个 app 结果（对齐 Burrow AppUpdateItem 网络部分） */
export interface AppCheckResult {
  path: string
  source: string
  latest_version: string | null
  page_url?: string | null
  minimum_os?: string | null
}

/** mole_updates_check 返回 */
export interface UpdatesCheckResult {
  checked_at: string
  /** 当前 macOS 版本，前端 OSUpdateGate 用 */
  running_os: string
  apps: AppCheckResult[]
  brew: BrewOutdatedItem[]
}

/** updates::brew-progress 事件 payload */
export interface BrewProgressEvent {
  /** 升级目标：单包 = name，全部 = "brew" */
  id: string
  phrase: string
}

// ============================================================
// scan_home / scan_directory (Rust native scanner)
// ============================================================

export interface ScanLargeFile {
  path: string
  size: number
}

export interface ScanNode {
  path: string
  name: string
  size: number
  is_dir: boolean
  is_folded: boolean
  children: ScanNode[]
}

export interface RuleItemBlueprint {
  id: string
  path: string
  title: string
  tips: string
}

export interface RuleCategoryBlueprint {
  id: string
  title: string
  tips: string
  items: RuleItemBlueprint[]
}

export interface ScanResult {
  root: string
  files_scanned: number
  dirs_seen: number
  bytes_total: number
  largest_files: ScanLargeFile[]
  items: ScanNode[]
  walk_errors: number
  folded_dirs: number
  folded_bytes: number
  size_metric: string
  rule_categories: RuleCategoryBlueprint[]
}

// ============================================================
// mole_clean_paths — Rust trash crate 直接移到废纸篓
// ============================================================

export interface MoleCleanPathItem {
  item_id: string
  category_id: string
  path: string
  size: number
}

export interface MoleCleanPathsResult {
  results: {
    category_id: string
    item_id: string
    path: string
    size_cleaned: number
    status: string
    error?: string | null
  }[]
  summary: {
    total_cleaned_size: number
    success_count: number
    failed_count: number
    free_space_change?: number
    free_space_change_human?: string
  }
}

// ============================================================
// Installer types (Rust native)
// ============================================================

export interface InstallerFile {
  path: string
  name: string
  size: number
  source_dir: string
}

export interface InstallerScanResult {
  files: InstallerFile[]
  total_size: number
  total_files: number
}

// ============================================================
// Touch ID types
// ============================================================

export interface TouchIdStatus {
  enabled: boolean
  supported: boolean
}

// ============================================================
// Whitelist types
// ============================================================

export interface WhitelistData {
  patterns: string[]
  warnings?: string[]
}

export interface WhitelistPredefinedItem {
  id: string
  title: string
  pattern: string
  category: string
}

// ============================================================
// Purge Paths types
// ============================================================

export interface PurgePathsData {
  paths: string[]
}

// ============================================================
// Startup types (对齐 Launchdeck model + Lemon App 分组)
// ============================================================

export type ServiceSource = 'launchd' | 'homebrew' | 'both'
export type ServiceScope = 'user_agent' | 'global_agent' | 'system_daemon'
export type ServiceStatus = 'running' | 'scheduled' | 'stopped' | 'failed' | 'unloaded' | 'disabled' | 'unknown'
export type SafetyLevel = 'user_writable' | 'admin_required' | 'readonly_system' | 'protected_vendor'
export type Provenance = 'homebrew' | 'user_plist' | 'vendor_app' | 'system' | 'runtime_only' | 'unknown'
export type EnableStatus = 'all_enabled' | 'some_enabled' | 'all_disabled'

/** 工具栏筛选 */
export type StartupFilter = 'all' | 'apps' | 'services' | 'problems'

export interface CalendarSchedule {
  minute: number | null
  hour: number | null
  day: number | null
  weekday: number | null
  month: number | null
}

export interface LaunchConfig {
  program: string | null
  arguments: string[]
  working_directory: string | null
  stdout_path: string | null
  stderr_path: string | null
  run_at_load: boolean | null
  keep_alive: string | null
  start_interval: number | null
  start_calendar_intervals: CalendarSchedule[]
}

export interface Origin {
  kind: Provenance
  confidence: 'high' | 'medium' | 'guess'
  evidence: string[]
}

export interface ElevationNeeds {
  runtime: boolean
  plist_write: boolean
  plist_remove: boolean
}

export interface AppAssociation {
  app_name: string
  app_path: string
  bundle_id: string
}

export interface StartupService {
  id: string
  label: string
  display_name: string
  source: ServiceSource
  scope: ServiceScope
  domain: string
  plist_path: string | null
  config: LaunchConfig
  pid: number | null
  exit_code: number | null
  status: ServiceStatus
  enabled: boolean | null
  loaded: boolean | null
  brew_formula: string | null
  brew_status: string | null
  safety_level: SafetyLevel
  elevation: ElevationNeeds
  origin: Origin
  app_info: AppAssociation | null
  health: string[]
}

export interface AppGroup {
  app_name: string
  app_path: string
  bundle_id: string
  services: StartupService[]
  enable_status: EnableStatus
}

export interface StartupInventory {
  app_groups: AppGroup[]
  standalone_services: StartupService[]
  warnings: string[]
}

export interface StartupActionResult {
  success: boolean
  message: string
}
