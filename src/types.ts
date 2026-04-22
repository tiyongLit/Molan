export type Controller = Record<string | symbol, any>

export enum AppTheme {
  System = 'system',
  Light = 'light',
  Dark = 'dark'
}

export enum AppLanguage {
  System = 'system',
  ZHCN = 'zh-CN',
  ZHTW = 'zh-TW',
  EN = 'en'
}

export enum Placements {
  Top = 'top',
  TopLeft = 'topLeft',
  TopRight = 'topRight',
  Bottom = 'bottom',
  BottomLeft = 'bottomLeft',
  BottomRight = 'bottomRight'
}

export interface Rectangle {
  // Docs: https://electronjs.org/docs/api/structures/rectangle

  /**
   * The height of the rectangle (must be an integer).
   */
  height: number
  /**
   * The width of the rectangle (must be an integer).
   */
  width: number
  /**
   * The x coordinate of the origin of the rectangle (must be an integer).
   */
  x: number
  /**
   * The y coordinate of the origin of the rectangle (must be an integer).
   */
  y: number
}

export interface AppStore {
  // Local storage address
  local: string
  // Download completion tone
  promptTone: boolean
  // Proxy address
  proxy: string
  // Whether to enable agent
  useProxy: boolean
  // Delete the original file after downloading
  deleteSegments: boolean
  // A new window opens the browser
  openInNewWindow: boolean
  mainBounds?: Rectangle
  browserBounds?: Rectangle
  blockAds: boolean
  // theme
  theme: AppTheme
  // Using browser plugins
  useExtension: boolean
  // Whether to use mobile UA
  isMobile: boolean
  // Maximum number of simultaneous downloads
  maxRunner: number
  // Language
  language: AppLanguage
  notifyPlacement: Placements
  // Show terminal or not
  showTerminal: boolean
  // Privacy mode
  privacy: boolean
  // Machine id
  machineId: string
  // Download proxy Settings
  downloadProxySwitch: boolean
  // Automatic update
  autoUpgrade: boolean
  // beta versions are allowed
  allowBeta: boolean
  // Close the main window
  closeMainWindow: boolean
  // Whether to play sounds in the browser. The default value is mute
  audioMuted: boolean
  // Whether to enable Docker
  enableDocker: boolean
  // Docker URL
  dockerUrl: string
}

/** 单条目标（网站或应用），一条规则可包含多个 targets */
export interface BrowserRuleTarget {
  targetType: 'website' | 'application'
  pattern: string
  patternType?: string
  patternLayer1?: string
  patternLayer3?: string
  isRegex?: boolean
  blockMethod?: 'close_tab' | 'open_new_tab' | 'go_to_home' | 'minimize' | 'close' | 'kill'
  blockingMode?: 'block_some' | 'block_all'
  matchScope?: 'page' | 'site'
  patternBlockPage?: string
  patternBlockSite?: string
  /** 应用规则：可先空串，与 Core 对齐 */
  sha256?: string
  signIssuer?: string
}

export interface BrowserRuleRequest {
  /** 编辑时传入，有 id 则走更新逻辑 */
  id?: string
  name: string
  /** 有 targets 时以 targets 为准；无 targets 时用 pattern/ruleType（兼容旧请求） */
  pattern?: string
  ruleType?: 'website' | 'application' | 'wildcard'
  description?: string
  planType?: 'focus_session' | 'alarm' | 'pomodoro'
  targetUserId?: string | null
  patternType?: string
  patternLayer1?: string
  patternLayer3?: string
  isRegex?: boolean
  blockMethod?: 'close_tab' | 'open_new_tab' | 'go_to_home' | 'minimize' | 'close' | 'kill'
  blockingMode?: 'block_some' | 'block_all'
  matchScope?: 'page' | 'site'
  patternBlockPage?: string
  patternBlockSite?: string
  /** 一条规则下的多个目标（网站+应用可同时存在）；非空时以 targets 为准 */
  targets?: BrowserRuleTarget[]
  isEnabled?: boolean
  isPaused?: boolean

  // 通知设置
  notifyStatusChange?: boolean
  notifySoonStarting?: boolean
  notifySoonEnding?: boolean
  notifyFixedIntervals?: boolean
  showTimer?: boolean
  idleTimeoutEnabled?: boolean

  // 惩罚机制
  punishMeWebsitesEnabled?: boolean
  numberOfBlocks?: number
  reoffenceWindowMinutes?: number
  punishmentDurationMinutes?: number

  // 时间计划（与 time_schedule 表字段一致）
  timeSchedule?: {
    scheduleType: 'alarm' | 'focus_session' | 'pomodoro'
    alarmSubType?: 'once' | 'daily' | 'weekday' | 'custom'
    startTime?: string
    durationMinutes?: number
    daysOfWeek?: string
    schedulePeriods?: string | null
    isActive?: boolean

    // 专注会话
    isDelayEnabled?: boolean
    delayMinutes?: number
    isBreakEnabled?: boolean
    breakDurationMinutes?: number
    showStartDialog?: boolean
    reshowOnStop?: boolean
    continueOnSleepShutdown?: boolean

    // 番茄钟
    pomodoroEnabled?: boolean
    pomodoroRounds?: number
    focusDurationMinutes?: number
    shortBreakDurationMinutes?: number
    currentPomodoroRound?: number
  }

  // 防护级别
  protectionLevel?: {
    protectionMode: 'none' | 'random_chars' | 'custom_password' | 'forced'
    activeStopChallenge?: 'none' | 'random_chars' | 'custom_password' | 'forced'
    activeStopDelayMinutes?: number
    nonActiveStopChallenge?: 'none' | 'random_chars' | 'custom_password' | 'forced'
    nonActiveStopDelayMinutes?: number
    pausingEnabled?: boolean
    addToAllowListEnabled?: boolean
    passwordHash?: string
    passwordLength?: number
    isUninstallProtected?: boolean
    isRebootProtected?: boolean
    thirdPartyPasswordEnabled?: boolean
  }

  // 使用限制
  usageLimit?: {
    limitType:
      | 'time_daily'
      | 'time_weekly'
      | 'time_monthly'
      | 'time_per_interval'
      | 'launch_frequency'
    maxValue: number
    intervalUnit?: 'hour' | 'minute'
    intervalValue?: number
    maxLaunchesPerDay?: number
    maxDurationPerSession?: number
    inactivityTimeoutMinutes?: number
    minIntervalBetweenLaunches?: number
  }

  // 白名单规则
  whitelistRules?: Array<{
    pattern: string
    isRegex?: boolean
    ruleScope: 'subdomain' | 'path' | 'query_param' | 'full_url'
    description?: string
  }>
}

export interface PaginationRequest {
  q?: string
  current: number
  pageSize: number
  sort?: 'ASC' | 'DESC'
}

export interface InterceptPlan {}

/** 列表/详情中时间计划字段（与后端 time_schedule 表对应，用于展示剩余时间、周期等） */
export interface TimeScheduleDisplay {
  scheduleType?: 'alarm' | 'focus_session' | 'pomodoro'
  durationMinutes?: number | null
  /** 专注会话实际开始时间（ISO 字符串），用于计算剩余时间 */
  sessionStartedAt?: string | null
  alarmSubType?: 'once' | 'daily' | 'weekday' | 'custom' | null
  startTime?: string | null
  schedulePeriods?: string | null
  daysOfWeek?: string | null
  pomodoroRounds?: number | null
  focusDurationMinutes?: number | null
  shortBreakDurationMinutes?: number | null
  currentPomodoroRound?: number
}

/** 接口返回的单条目标（与 backend BlockingTarget 对应） */
export interface PlanRuleTarget {
  id?: string
  targetType?: 'website' | 'application'
  pattern?: string
  patternType?: string
  patternLayer1?: string
  patternLayer3?: string
  blockMethod?: string | null
  blockingMode?: string
  matchScope?: string
  patternBlockPage?: string
  patternBlockSite?: string
}

/** 分页接口返回的单条规则（含关联表，供 Control 等列表使用） */
export interface PlanRuleWithRelations {
  id: string
  name: string
  description?: string | null
  planType?: 'focus_session' | 'alarm' | 'pomodoro' | null
  targetUserId?: string | null
  ruleType: 'website' | 'application' | 'wildcard'
  pattern: string
  blockMethod?: string | null
  blockingMode?: string
  matchScope?: 'page' | 'site'
  patternBlockPage?: string
  patternBlockSite?: string
  /** 一条规则下的多个目标（网站+应用），编辑回显用 */
  targets?: PlanRuleTarget[]
  isEnabled: boolean
  isPaused?: boolean
  createdAt?: string
  updatedAt?: string
  timeSchedule?: TimeScheduleDisplay | Record<string, unknown> | null
  protectionLevel?: Record<string, unknown> | null
  usageLimit?: Record<string, unknown> | null
  whitelistRules?: unknown[]
  emergencyAccessLogs?: unknown[]
}

/** Rust `emit_table_refresh` 与前端订阅约定 */
export interface TableRefreshPayload {
  keys: string[]
  timestamp_ms: number
  source: string
}

export interface PlanRulePageResult {
  total: number
  rules: PlanRuleWithRelations[]
}

declare global {
  type Nullable<T> = T | null
  type Maybe<T> = T | null | undefined
  type Optional<T> = T | undefined
  type TruthyString = 'true' | 'false'
}
