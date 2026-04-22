# 更新（Updates）功能 — 移植方案与实现设计（评审稿）

> 目的：把 Burrow 的 Updates 标签页后端逻辑移植到 MoleStudio2（Tauri 2 + Rust）。
> 本文件是**设计方案**，含架构决策、命令契约、语义对齐清单与实现顺序；评审通过后按第 7 节执行。
>
> 关键前提：**此功能与 Mole 无对齐关系**。Mole CLI 没有应用更新检查（`mo update` 只更新 Mole 自身）。
> Burrow 的更新逻辑是它自己的 Swift 原创，唯一复用 Mole 的部分是应用清单（`mo uninstall --list`），
> 而那份清单我们已有等价物 `mole_list_apps`。因此这是「移植 Burrow 原创逻辑」，不是「对齐 Mole」。

参考实现（行为权威）：
- `Burrow/macos/Sources/UpdatesView.swift`（UpdatesModel，交互与状态机）
- `Burrow/macos/Sources/UpdateSources.swift`（detect / feedURL / appcast / iTunes 解析）
- `Burrow/macos/Sources/UpdateCheck.swift` L386-400（isNewer 版本比较）
- `Burrow/macos/Sources/OSUpdateGate.swift`（App Store 更新与当前 macOS 的兼容门）
- `Burrow/macos/Sources/BrewProgress.swift`（brew 输出 → 进度短语）

---

## 1. 结论摘要

| 维度 | 结论 |
|---|---|
| 可行性 | ✅ 可移植，且比卸载 tab 简单得多（纯检测 + 网络 + 深链，无扫描残留的复杂度） |
| 输入数据 | ✅ 100% 覆盖：`mole_list_apps` 返回的 `MoleListAppsEntry` 已含 `version / bundle_id / size_human / last_used_epoch / source / uninstall_name`（Burrow 还要惰性读 Info.plist 拿版本，我们列表里直接有） |
| 新依赖 | ✅ 零新增 crate：网络用系统自带 `curl`；XML 用 `quick-xml 0.41`（已在 Cargo.lock，是 `plist` 的传递依赖，提为直接依赖即可，零新编译） |
| 规模 | 约 500 行 Rust（含探针测试）+ 300 行 TS |

---

## 2. Burrow 行为基线（移植权威语义）

### 2.1 生命周期

```
进入页面
  → prepare()      检测每个 app 的更新源（纯本地 fs 检查，零网络）
  → autoSurface()  每会话一次：brew outdated 自动出（低敏感，用户自己的工具）
  → [用户点击 Check]
  → checkNow()     并发 6 发网络请求（appcast / iTunes）+ 刷新 brew 行
  → update()/upgrade()  深链更新操作 / brew 流式升级
```

### 2.2 各方法语义（UpdatesView.swift 实测）

| 方法 | 行为 |
|---|---|
| `prepare(apps)` | 逐 app `UpdateSources.detect`：可检测 → `appItems`；不可检测 → `uncheckableApps`。幂等：`apps.count` 不变不重跑 |
| `autoSurface()` | guard `!brewSurfaced && !checking`；置 `brewSurfacing` 指示器；`brewOutdated()` 完成后**仅当 brew 行为空时写入**；会话内只跑一次 |
| `checkNow()` | guard `!checking`；TaskGroup 并发上限 6；app 检查全部完成后 `brewOutdated()` 刷新 brew 行（必刷新）；`checked=true`；另起低优先级任务用 fs 日期回填 lastUsed |
| `check(item)` | sparkle → feedURL + fetch + parseAppcast；appStore → iTunes lookup（bundleID 为空则跳过）；**electron/homebrew → 不请求**（v1 只打徽标，它们自带更新器） |
| `fetch(url)` | timeout 10s；`reloadIgnoringLocalCacheData`（手动检查 = 拿新鲜元数据） |
| `update(item)` | sparkle/electron → `openApplication(path)`；appStore → 有 pageURL 开 pageURL，否则开 `macappstore://showUpdatesPage`；homebrew → 无操作 |
| `upgrade(item)` | guard `upgrading` 集合（brew 自带锁，禁止并发）；`brew upgrade <name>` 流式 1800s；逐行 `BrewProgress.phrase`（`==> ` 前缀行）→ `brewPhrase`；结束后刷新 brew 行 |
| `upgradeAll()` | `brew upgrade` 流式 3600s，其余同上 |

### 2.3 UI 分区规则

- **未 checked**：显示 "Apps with an update mechanism" = `appItems`（含 Electron 徽标行）+ brew 行（Homebrew 徽标只出现在 brew 行，**不标应用行**）。
- **checked 后**：
  - `available`：`latestVersion != nil && isNewer(latest, installed)`
  - `upToDate`：`latestVersion != nil && !isNewer`
  - **Electron 行消失**：source 有值但 `latestVersion == nil`，既非 available/upToDate，也非 uncheckable（其自带更新器接管，查无可查）
  - `uncheckable`：`detect` 返回 nil 的 app
- App Store 行另有 `OSUpdateGate`：更新要求的 macOS 高于当前系统时，该行不显示（不可安装）。

---

## 3. 架构决策（已与用户确认，不可再改）

| # | 决策 | 结论 | 理由 |
|---|---|---|---|
| D1 | 网络层 | **Rust 命令内 `curl` 子进程**，不加任何 HTTP 库 | 沿用 MoleStudio 现有约定（所有外部能力走 Tauri 命令 + useTauri IPC）；零依赖；`-o 临时文件`天然规避管道死锁 |
| D2 | 检测落点 | **`mole_list_apps` 返回加 `update_source` 字段** | 对齐 Burrow `prepare()` 一次性检测（纯本地、极快），切到更新 tab 零额外 IPC |
| D3 | XML 解析 | `quick-xml`（提为直接依赖） | 已在 lock 中（plist 传递依赖），零新编译；只做 `<enclosure>` 属性提取 |
| D4 | 版本比较 | **新写 `is_version_newer`**，不复用 `lib/clean/user.rs::version_compare` | 语义不同（见 §6.1），复用会出 bug |
| D5 | brew 输出捕获 | 临时文件（outdated）/ 排空线程（upgrade 流式） | `lib/core/timeout.rs::run_with_timeout_capture` 有 64KB 管道死锁 bug（卸载 tab 根因）；顺手修掉 |
| D6 | 「每会话一次」guard | 前端 React state | Burrow 是 model 属性，前端等价物；后端命令保持无状态 |

---

## 4. 后端设计

### 4.1 模块结构（新增 `lib/updates/`，与 `lib/uninstall/` 平级）

| 文件 | 职责 | 对应 Burrow |
|---|---|---|
| `lib/updates/mod.rs` | 公共类型 `UpdateSource`、`OutdatedItem`、`UpdateCheckItem` | — |
| `lib/updates/detect.rs` | `detect_update_source(path)`：3 个 fs 检查 | `UpdateSources.detect` |
| `lib/updates/appcast.rs` | `feed_url(path)`（读 Info.plist `SUFeedURL`）+ `parse_appcast(xml)` | `UpdateSources.feedURL/parseAppcast` |
| `lib/updates/itunes.rs` | `itunes_lookup(bundle_id)` + `parse_itunes_lookup(json)` | `UpdateSources.parseITunesLookup` |
| `lib/updates/version.rs` | `is_version_newer(remote, local)` + `os_is_installable(minimum, running)` | `UpdateCheck.isNewer` + `OSUpdateGate` |
| `lib/updates/brew.rs` | `brew_path()`、`brew_outdated()`、`brew_upgrade_streaming()` | `UpdatesModel.brewOutdated/upgrade/upgradeAll` |
| `controllers/updates.rs` | Tauri 命令薄层（4 个命令） | — |

### 4.2 数据结构（serde 默认 snake_case，与前端 camelCase 由 Tauri 映射）

```rust
pub enum UpdateSource { Sparkle, AppStore, Electron }   // JSON: "sparkle" | "app_store" | "electron"
// mole_list_apps 的 AppListEntry 增加字段：
//   update_source: Option<UpdateSource>   // None = 不可检测

pub struct BrewOutdatedItem {
    pub name: String,        // formulae / casks 的 name
    pub installed: String,   // installed_versions 最后一项
    pub latest: String,      // current_version
    pub kind: String,        // "formula" | "cask"
}

pub struct AppCheckResult {
    pub path: String,
    pub source: String,                        // "sparkle" | "app_store" | "electron"
    pub latest_version: Option<String>,        // 请求失败/electron → null（静默）
    pub page_url: Option<String>,              // 仅 app_store
    pub minimum_os: Option<String>,            // 仅 app_store
}

pub struct UpdatesCheckResult {
    pub checked_at: String,                    // ISO8601
    pub apps: Vec<AppCheckResult>,
    pub brew: Vec<BrewOutdatedItem>,
}
```

### 4.3 命令契约（5 命令 + 1 事件）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `mole_list_apps` | 无 | 现有结构 + `update_source` | 已有命令，仅扩展字段 |
| `mole_updates_brew_outdated` | 无 | `{ items: BrewOutdatedItem[] }` | autoSurface 专用；brew 不存在 → 空数组 |
| `mole_updates_check` | `app_paths: Vec<String>` | `UpdatesCheckResult` | 用户点击 Check；并发 6；app 检查完后 brew outdated（必刷新） |
| `mole_updates_apply` | `action: "open_app" \| "open_url" \| "macappstore"`, `target: Option<String>` | `{ ok: bool, error: Option<String> }` | 深链：`open <path>` / `open <url>` / `open macappstore://showUpdatesPage` |
| `mole_updates_brew_upgrade` | `name: Option<String>` | `{ ok: bool, exit_code: Option<i32>, error: Option<String> }` | None = 全部；进度走事件 |
| 事件 `updates::brew-progress` | — | `{ id: String, phrase: String }` | 流式逐行推；`phrase` = 行 `strip_prefix("==> ")` 后的内容 |

### 4.4 核心函数规格

- `detect_update_source(path) -> Option<UpdateSource>`
  1. `<app>/Contents/_MASReceipt/receipt` 存在 → AppStore
  2. Info.plist 含 `SUFeedURL` → Sparkle（复用现有 plist 读取方式，与列表扫描一致）
  3. `<app>/Contents/Frameworks/Electron Framework.framework` 存在 → Electron
  4. 否则 None
- `is_version_newer(remote, local) -> bool`（逐条对齐 `UpdateCheck.isNewer`）
  1. trim 两端空白；剥离**一个**前导 `v`/`V`
  2. 按 `.` 拆分；每段 `parse::<i64>().unwrap_or(0)`（非数字段归 0，`"2024b"` → 0）
  3. 缺段补 0 逐段比；任一段不等即返回 `remote > local`
  4. 全等返回 `false`
- `parse_appcast(xml: &str) -> Option<String>`：quick-xml Reader 遍历，`<enclosure>` 元素属性优先取 `sparkle:shortVersionString`，无则取 `sparkle:version`（属性名带前缀，按字面匹配）。
- `curl` 调用规格：`curl -sSL --max-time <N> -o <tmpfile> <url>`；tmpfile 用 `temp_dir()` + pid/时间戳唯一名，读完即删；任何失败（非零退出/文件缺失/解析失败）→ 静默返回 None（对齐 Burrow `try?` + `guard let else` 语义，行保留原状）。
  - appcast / iTunes：`--max-time 10`
  - brew outdated：`--max-time 120`（不用 curl，直接 `brew` 命令落临时文件）

---

## 5. 前端设计

### 5.1 类型（`src/types/mole.ts`）

```ts
export interface MoleListAppsEntry { /* 现有字段 */ update_source: 'sparkle' | 'app_store' | 'electron' | null }
export interface BrewOutdatedItem { name: string; installed: string; latest: string; kind: string }
export interface AppCheckResult { path: string; source: string; latest_version: string | null; page_url?: string | null; minimum_os?: string | null }
export interface UpdatesCheckResult { checked_at: string; apps: AppCheckResult[]; brew: BrewOutdatedItem[] }
export type UpdatesApplyAction = 'open_app' | 'open_url' | 'macappstore'
export interface BrewProgressEvent { id: string; phrase: string }
```

### 5.2 UpdatesTab 状态机（对齐 UpdatesModel）

| 状态 | 触发 | 行为 |
|---|---|---|
| `prepared` | `apps` prop 到达（壳层已有列表） | 按 `update_source` 分区：可检测行 / uncheckable 行；`apps.length` 不变不重跑 |
| `brewSurfacing` | 首次挂载（session guard） | 调 `mole_updates_brew_outdated`；期间 header 显示 "Checking Homebrew…"；结果仅当 brew 行为空时写入 |
| `checking` | 用户点 Check | 调 `mole_updates_check({ app_paths })`；header 换 "Checking…"；结果整体替换 app 行 + brew 行 |
| `checked` | check 完成 | 分区切换为 available / upToDate / brew / uncheckable；Electron 行隐藏 |
| `apply` | 行内 Update 按钮 | sparkle/electron → `mole_updates_apply('open_app', path)`；app_store → 有 page_url 开它，否则 `macappstore`；brew → 无 |
| `upgrading` | brew 行 Upgrade 按钮 | `mole_updates_brew_upgrade({ name })`；`onIpcEvent('updates::brew-progress')` 渲染短语；完成刷新 brew 行；id 集合防并发 |

### 5.3 壳层与其余改动

- `index.tsx`：`<UpdatesTab apps={apps} />`（Burrow 的 UpdatesView 同样是拿 apps 数组）；顶部右侧"检查更新会访问 Apple 与厂商服务器"提示文案已就位，不动。
- `mock.ts`：删除 updates 部分（startup 仍 mock）。
- `constants/tauri-commands.ts`：+4 个命令名；`constants/tauri-events.ts`：+`EVT_UPDATES_BREW_PROGRESS`。

### 5.4 mock 与 Burrow 的差异修正

现 mock 中 Homebrew 徽标出现在**应用行**上；Burrow 只在 **brew 行**显示 homebrew 徽标。接真实数据时按 Burrow 修正。

---

## 6. 语义对齐清单（实现时逐条对照）

| # | Burrow 行为 | 实现落点 |
|---|---|---|
| 1 | `isNewer`：去一个前导 v/V、非数字段归 0、缺段补 0、全等 false | `version.rs::is_version_newer`；**禁止**复用 `clean/user.rs::version_compare`（sort -V 语义：不剥 v、非数字段字典序、不等长直接比长度，三处都不同） |
| 2 | 检测三来源顺序：MAS receipt → SUFeedURL → Electron Framework | `detect.rs` 同序 |
| 3 | Electron 行 checked 后从列表消失（source 有值、latestVersion=null） | 前端分区：`latest_version != null` 才进 available/upToDate，source=null 进 uncheckable |
| 4 | 请求失败静默：latestVersion 保持 null，行保留 | curl 失败 → None，不报错 |
| 5 | fetch 超时 10s、忽略本地缓存 | `--max-time 10`（curl 无缓存，天然满足） |
| 6 | appcast 常 302 | curl 必须 `-L` |
| 7 | brew outdated 120s 级慢操作 | 独立超时 + `brewSurfacing` 占位 |
| 8 | brew 自带锁，禁止并发 upgrade | 前端 `upgrading` id 集合 guard（后端也可加防重入静态集，双保险） |
| 9 | 单包 1800s / 全部 3600s | `mole_updates_brew_upgrade` 超时参数 |
| 10 | `brewPhrase` = `==> ` 前缀行剥前缀 | `line.strip_prefix("==> ")`，非 `==>` 行（进度条/空行）丢弃 |
| 11 | OSUpdateGate：minimumOs > 当前系统 → 行不显示 | `sw_vers -productVersion` + 点分比较（`version.rs::os_is_installable`） |
| 12 | iTunes：bundleID 为空跳过；`resultCount=0` → None | `itunes.rs` |
| 13 | autoSurface 结果仅当 brew 行为空时写入 | 前端同判 |
| 14 | checkNow 必刷新 brew 行（app 检查完成后） | `mole_updates_check` 内串行：并发 app 检查 → 完成后 brew outdated |
| 15 | 隐私：appcast/iTunes 仅用户点击后请求；brew outdated 自动（用户自己的工具） | 命令入口仅由前端上述状态机触发 |

---

## 7. 实现顺序（评审通过后执行）

1. **后端纯函数 + 探针测试**：`lib/updates/{detect,version,appcast,itunes,brew}.rs` 全写完，`tests/updates_probe.rs` 探针（沿用卸载 tab 打法，stderr logger + `--nocapture`）：
   - `probe_detect_sources`：构造临时 `.app` 目录，三来源各验一遍
   - `probe_version_is_newer`：v 前缀 / 非数字段 / 不等长 / 相等 / `2024b`
   - `probe_appcast_parse`：真实 appcast XML 片段 + 无 `shortVersionString` 回退 `sparkle:version`
   - `probe_itunes_parse`：`resultCount=0` / 正常返回
   - `probe_brew_outdated`：`brew outdated --json=v2` 真实输出解析（无 brew 环境则 mock 字符串）
2. **命令层**：`controllers/updates.rs` 4 个命令 + `lib.rs` invoke_handler 注册 + `events.rs` 加 `EVT_UPDATES_BREW_PROGRESS` 与 payload + emit 助手。
3. **list_apps 扩展**：`controllers/uninstall.rs` 的列表条目加 `update_source`（调用 `detect_update_source`）。
4. **顺手修 `timeout.rs` 管道死锁**：fallback 分支改排空线程或临时文件（卸载 tab 根因，一劳永逸）。
5. **前端**：`types/mole.ts` + `constants/tauri-commands.ts` + `constants/tauri-events.ts` → `index.tsx` 传 apps → `UpdatesTab.tsx` 重写接真实数据 → 删 `mock.ts` updates 部分。
6. **冒烟**（真实环境）：一个 Sparkle app（如 DBX/VS Code）+ 一个 MAS app + 一个 brew cask + 一个 Electron app；验证：徽标、checked 后分区、Electron 消失、点击 Update 拉起对应更新器、brew 升级流式短语、断网时静默保留行。

---

## 8. 风险与已知坑

1. **管道死锁**（已实锤的坑）：所有 shell 输出捕获禁止复用 `run_with_timeout_capture` 的 wait-then-read 模式；本次实现全部走临时文件 / 排空线程，并顺手修 timeout.rs。
2. **brew outdated 可达 120s**：autoSurface 期间必须有 "Checking Homebrew…" 占位，否则用户以为卡死。
3. **curl 无重定向默认**：忘记 `-L` 会让大量 appcast 静默失败（302 → 空文件 → None），症状是"全部无更新"，难排查。
4. **`open macappstore://showUpdatesPage`** 在部分系统版本行为不一致：失败不致命（返回 ok=false 即可），不阻塞 UI。
5. **升级中用户切走 tab**：`updates::brew-progress` 事件监听随组件卸载销毁；后端子进程照常跑完，前端回来自动刷新 brew 行（autoSurface 已消耗时需靠行内刷新按钮，见 5.2 `upgrading` 完成路径）。
6. **并发 check 的 curl 进程数**：6 个并发上限与 Burrow 一致，避免压垮弱网络。
