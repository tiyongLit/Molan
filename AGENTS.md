# AGENTS.md — Molan 项目 AI 规则指引

> 本文件是 `.trae/rules/` 目录下所有规则文件的融合版本，供 AI Agent 与开发者一次性阅读。  
> 如与分文件版本冲突，以本文件为准。

---

## 目录

1. [项目身份与技术栈](#1-项目身份与技术栈)
2. [参考项目分工](#2-参考项目分工)
3. [硬约束与红线（最高优先级）](#3-硬约束与红线最高优先级)
4. [后端工作流（Rust / Tauri）](#4-后端工作流rust--tauri)
5. [前端工作流（React / TypeScript）](#5-前端工作流react--typescript)
6. [统计口径与双版本构建](#6-统计口径与双版本构建)
7. [架构约束（待修订）](#7-架构约束待修订)

---

## 1. 项目身份与技术栈

### Molan 是什么

Molan 是一款 **macOS 清理 / 优化 GUI 软件**，面向最终用户上架 Mac App Store（MAS）与官网分发。

它是 Molan（v1 MVP）的第二版。v2 在 v1 已验证的 Rust 后端底座之上重做前端 UI，并补充功能。

### 与 v1 的关系

- **后端继承**：v2 的 `src-tauri/` 基本沿用 v1 的 Rust 实现（`controllers/`、`lib/`、`embedded_rules.rs`），并持续迭代。
- **前端重构**：v2 引入新的外壳布局（`Shell*` 组件）与视觉体系，替换 v1 的页面组织方式。
- **架构约束延续**：v1 `docs/架构约束.md` 的核心红线在 v2 仍然适用，但部分细节待人工修订。

### 技术栈

**前端（`src/`）**

- React 19 + TypeScript（strict）
- React Router 7/8
- antd 6 + Tailwind CSS 4 + shadcn / base-ui
- framer-motion / motion / gsap / three / ogl（动效与可视化）
- lucide-react（图标）、ahooks、lodash-es
- Vite 7 构建
- 样式：SCSS module + Tailwind
- i18n：`src/i18n/locales/<lang>/translation.json`

**后端（`src-tauri/`）**

- Tauri 2 + Rust
- 命令入口：`src-tauri/src/controllers/`
- 底层能力：`src-tauri/src/lib/`（域模块：`clean`、`core`、`manage`、`optimize`、`uninstall`、`check`、`platform`、`startup`）
- 磁盘分析与系统监控：`src-tauri/src/lib/analyze/`、`src-tauri/src/lib/status/`（自 CLI `cmd/` 迁移归位）
- 内嵌规则：`src-tauri/src/embedded_rules.rs`（编译期 const/静态表）
- 事件：`src-tauri/src/events.rs`
- 运行时接线：`src-tauri/src/runtime/`（托盘 `tray`、应用菜单 `app_menu`、Dock 生命周期 `macos_dock_quit`、后台 watcher `residual_watch` / `trash_watch`）

### 前后端契约

- 后端 Tauri command 名 → 前端在 `src/constants/tauri-commands.ts` 登记为类型化常量。
- 后端事件名 → 前端在 `src/constants/tauri-events.ts` 登记。
- 数据结构 → 前端 `src/types/mole.ts`。
- **原生图标**：文件/应用图标统一走内容寻址注册表（IPC 命令 `mole_native_icons_resolve`），架构、使用规范与红线见 `docs/原生图标系统架构.md`；前端入口 `src/utils/nativeIconRegistry.ts`。
- **卸载残留通知**：设置项「自动检测卸载残留」的完整链路（原生通知、双通道送达、定向扫描、事件契约、排障手册）见 `docs/卸载残留通知与定向扫描方案总结.md`；全局状态入口 `src/layout/ResidualContext.tsx`。

### 项目根目录约定

- `src/`、`src-tauri/`：应用代码
- `docs/`：本项目自身文档
- `bg/`、`css3动效/`、`ui/`：**设计素材参考**，不属于应用代码，不要在其中实现业务逻辑
- `public/`：静态资源

### 应用功能域（参考 Mole CLI）

clean（深度清理）、uninstall（智能卸载）、optimize（优化维护）、analyze（磁盘透视）、status（实时状态）、purge（构建产物清理）、installer（安装包清理）。Molan 在 GUI 中以 `Shell*` 外壳组织这些域。

---

## 2. 参考项目分工

Molan 不是从零开始，有四个参考项目。每个项目角色清晰，**只参考其角色范围内内容，不跨界**。

| 项目 | 技术栈 | 角色 | 可参考 | 不可参考 |
|---|---|---|---|---|
| **Mole**（`tw93/mole`，`mo` CLI） | Go + Bash | **算法/扫描清理逻辑的权威参考** | 路径规则、清理分类、size 算法、protected 名单、命令行口径 | **不嵌入** Go 二进制或 Shell 脚本；逻辑全部 Rust 重写 |
| **lemon-cleaner**（腾讯 Lemon） | Objective-C / Cocoa | **UI/UX 交互基准** | 扫描入口、分类折叠面板、可展开目录、勾选+统计呈现 | 不参考其代码实现 |
| **CleanMyMac X** | 闭源 | 商业产品 UX 对照 | 功能流程、信息架构 | 不涉及代码 |
| **Molan v1** | Tauri + React + Rust | **已验证的 Rust 后端来源 + 架构约束出处** | Rust 后端实现、架构红线 | 前端页面组织已被 v2 替换 |

**一句话总结**：Mole 给"算什么"、Lemon 给"长什么样"、v1 给"已实现的底座和红线"、v2 在此之上重做 UI 并补功能。

### 参考时的强制动作

1. 涉及**扫描/清理/统计逻辑**：先查 Mole 对应实现看逻辑口径，再查 v1/v2 是否已有 Rust 实现。
2. 涉及**UI 交互/信息架构**：对照 lemon-cleaner 看交互范式，但不抄代码。
3. 任何参考都**不得引入与硬约束冲突**的实现。

---

## 3. 硬约束与红线（最高优先级）

> **本节约束为最高优先级，任何其他规则不得与之冲突。违反红线的实现一律拒绝。**

### 红线 1：不调用外部二进制或 Shell 脚本

**所有扫描、统计、清理、优化、卸载逻辑必须使用 Rust 原生实现。**

- 禁止嵌入或调用 Mole 的 `mo` / `mole` Go 二进制。
- 禁止 `Command::new("sh")` / `bash` / `osascript` 等执行外部脚本（除下文明确豁免）。
- 目的：满足 MAS 上架与公证要求，避免「动态可执行行为」审核风险。

**豁免**（须在代码与 commit 中说明理由）：
- Tauri 官方插件（`tauri-plugin-dialog` / `tauri-plugin-opener` 等）提供的系统能力。
- 通过 Rust crate（`core-foundation`、`objc2`）调用 macOS 系统 API，不算"外部二进制"。

### 红线 2：不把 Mole 当运行时嵌入

- Mole、Lemon 都只是**需求与对照基准**，在约束内用 Rust 重写。
- **不打包** Mole 的 Go 二进制。
- 禁止在 `src-tauri/` 中引入 Swift / Objective-C 源文件或 Xcode 工程。

### 红线 3：清理规则编译期内嵌

- 规则存于 `src-tauri/src/embedded_rules.rs`（`const` / 静态表 + 序列化输出），随版本在 Git 中审计。
- **不依赖**应用包外可改写的 `rules.yaml` / 下载规则热更新。
- 例外（若将来要做）：用户主动「导入」的自定义规则须严格校验、沙箱内仅影响用户显式授权路径，且不作为默认主路径。

> ⚠️ 待修订点：作者认为"严禁任何外置规则"可能过严，未来或允许受控的自定义规则导入。修订前按本节执行。

### 红线 4：清理走废纸篓 + 用户确认

- 清理统一走 `trash` crate（移入废纸篓），**不直接 `rm`**。
- 任何清理动作前必须有用户显式确认（前端勾选 + 确认按钮）。
- 不在沙箱应用内自动清理系统根路径。
- 不直接卸载 `/Applications` 下其他应用；卸载走"应用 + 残留"清单 + 用户确认。

### 红线 5：安全与权限

- 不在代码中硬编码、日志输出或提交任何密钥、证书、token。
- 高风险能力（清理系统路径、卸载他人应用）一律走用户确认与白名单（`whitelist`）。
- 涉及 Full Disk Access 等系统权限的能力，引导用户在系统设置中授权，**不**尝试绕过。

### 红线 6：依赖与供应链

- 新增 Rust crate 须评估沙箱兼容性与许可证（优先 MIT/Apache-2.0）。
- 新增 npm 依赖须在 `package.json` 中登记，优先成熟、活跃维护的包。
- 不引入与 Tauri 2 / React 19 不兼容的旧版依赖。

### 红线 7：不主动创建文档与文件

- 除用户明确要求外，**不主动**创建 `*.md`、`README.md` 等文档文件。
- 不主动 scaffold 不必要的脚手架文件；优先编辑现有文件而非新建。

---

## 4. 后端工作流（Rust / Tauri）

适用于 `src-tauri/` 下所有改动。

### 新增能力时的标准流程

1. **查逻辑口径**：先查 Mole 对应实现，理解路径规则、分类、算法、保护名单。
2. **查已有实现**：查 v2 与 v1 是否已有 Rust 版本，能复用就复用、能迭代就迭代。
3. **在 `lib/` 实现**：按 Mole 的 `lib/*` 模块边界落位。底层能力放 `lib/`，不放 `controllers/`。
4. **经 `controllers/` 暴露**：写 `#[tauri::command]` 函数，注册到 `lib.rs` 的 `invoke_handler`。
5. **登记前端契约**：前端在 `tauri-commands.ts` 增加类型化常量；数据结构同步到 `mole.ts`。
6. **事件**：若需流式进度，在 `events.rs` 定义事件名，前端在 `tauri-events.ts` 登记，用 `emit` 推送。
7. **规则**：涉及清理规则，进 `embedded_rules.rs`，**不外置**。

### 模块边界

| 目录 | 职责 |
|---|---|
| `src-tauri/src/lib/clean/` | 深度清理（caches、apps、brew、dev、system、user…） |
| `src-tauri/src/lib/core/` | 公共能力（base、common、file_ops、bundle_resolver、pkg_receipts、app_protection、sudo、timeout…） |
| `src-tauri/src/lib/manage/` | 白名单、清理路径、自更新、autofix |
| `src-tauri/src/lib/optimize/` | 优化维护（diagnostics、maintenance、tasks） |
| `src-tauri/src/lib/uninstall/` | 智能卸载（batch、brew） |
| `src-tauri/src/lib/check/` | 健康检查、安全检查、开发环境检查 |
| `src-tauri/src/lib/platform/` | macOS 平台能力（原生图标注册表 `native_icon_registry` + 编码管线 `macos_file_icon`、原生通知 `macos_notifications`、特权路由） |
| `src-tauri/src/lib/analyze/` | 磁盘透视引擎（原 cmd/analyze 迁移） |
| `src-tauri/src/lib/status/` | 实时状态采集引擎（原 cmd/status 迁移） |
| `src-tauri/src/runtime/` | GUI 运行时接线（tray、app_menu、macos_dock_quit、residual_watch、trash_watch） |
| `src-tauri/src/controllers/` | Tauri command 入口，薄层，只做参数解析与分发 |
| `src-tauri/src/events.rs` / `src-tauri/src/constants.rs` | 事件契约与共享常量（root 级） |
| `src-tauri/src/embedded_rules.rs` | 编译期内嵌清理规则 |

### 命令设计

- 命令名用 `snake_case`，前缀按域（如 `clean_scan`、`clean_apply`、`uninstall_scan`、`analyze_scan_home`）。
- 参数用结构体或具名参数，**不用**裸 `Vec<String>` 之类难维护的签名。
- 返回值用 `Result<T, String>`，错误信息可读、可国际化（前端按 code 映射 i18n）。
- 涉及清理的命令，**默认 dry-run 友好**：扫描与执行分离，扫描不产生副作用。

### 异步与性能

- **重 I/O 必须 `spawn_blocking`**（或等价异步隔离），避免阻塞 UI 线程。
- 整盘/深层遍历可上 `rayon` / `jwalk` 并行，目标秒级可感知响应。
- 长任务用 Tauri 事件推送进度，前端订阅，避免轮询。

### 清理安全

- 删除走 `trash` crate（移入废纸篓）。
- 受保护应用/路径走 `lib/core/app_protection.rs` 与 `whitelist`，**不**绕过。
- 高风险操作（系统路径、`/Applications` 卸载）必须用户显式确认。

---

## 5. 前端工作流（React / TypeScript）

适用于 `src/` 下所有改动。

### 目录结构与落位

| 目录 | 职责 |
|---|---|
| `src/layout/` | 应用外壳与全局布局（`Shell*`、`Dock`、`Sidebar`、`themeColors`） |
| `src/pages/<Feature>/` | 各功能域页面（Clean、Home、Analyze、Optimize、Uninstall） |
| `src/components/ui/` | 通用 UI 组件，统一 `Mole*` 前缀（`MoleButton`、`MoleCard`…） |
| `src/components/reactbits/` | 动效零件（`Radar`、`MagicRings`、`LightRays`…） |
| `src/components/business/` | 业务复合组件（`Clean/`、`DiskCard/`…） |
| `src/constants/` | Tauri 命令/事件名常量（`tauri-commands.ts`、`tauri-events.ts`） |
| `src/hooks/` | 自定义 hooks（`useTauri`、`useDiskStatus`、`useNativeIcon`…） |
| `src/types/` | 与后端共享的 TS 类型（`mole.ts`） |
| `src/utils/` | 工具函数（`format`、`platform`、`nativeIconRegistry`…） |
| `src/lib/` | 前端底层工具（`utils.ts`） |
| `src/i18n/locales/<lang>/translation.json` | 多语言文案 |

### 与后端通信

- **只走 `invoke` + 类型化 command 常量**：从 `tauri-commands.ts` 导入命令名，不手写字符串。
- 事件订阅用 `@tauri-apps/api/event` 的 `listen`，事件名从 `tauri-events.ts` 导入。
- 数据结构以 `mole.ts` 为准，与后端 `serde` 结构保持一致。
- 异步状态优先用 `ahooks`（`useRequest` 等）或自建 hook，避免组件内散落 `useState` + `useEffect`。

### 原生图标系统（统一口径）

- **文件/应用图标一律走 `nativeIconRegistry`**：渲染用 `NativeIcon` / `AppIcon` / `useNativeIcon` / `useNativeIconMap`，批量预取用 `resolveIdle`；emoji 仅作未命中降级（`staticIconMap`），Clean / Optimize 的 ant-design 分类装饰图标不在此列。
- **禁止**新引入按路径 PNG base64 管线、独立图标缓存或绕过注册表的 `iconForFile` 调用；symlink 路径必须原样传入（不得 `canonicalize`，否则丢 alias 角标）。
- 新增图标消费点前先读 `docs/原生图标系统架构.md`。

### 页面与外壳

- 顶层路由在 `src/routers.tsx`，页面以 `Shell*` 外壳组织。
- 全局上下文：`ScanButtonContext`、`ScanSessionsContext`、`ShellNavContext`、`useBackgroundGradient`、`ResidualContext`（卸载残留定向目标，见 `docs/卸载残留通知与定向扫描方案总结.md`）。
- 页面切换动效走 `PageTransition` / `PageWrapper`。

### 组件命名与样式

- 通用 UI 组件统一 `Mole*` 前缀，放 `components/ui/`。
- 业务组件放 `components/business/<域>/`。
- 样式：SCSS module（`*.module.scss`）+ Tailwind 优先；复杂动效用 framer-motion / motion / gsap / three / ogl。
- 主题色集中在 `src/theme.css` 与 `src/layout/themeColors.ts`，**不**在组件内硬编码颜色。

### i18n

- 所有用户可见文案走 i18n，key 用点分命名（`clean.scan.button`）。
- 新增语言在 `src/i18n/locales/` 下建 `<lang>/translation.json`。

### 交互基准（对齐 Lemon）

- 扫描入口、分类折叠面板、可展开目录、勾选 + 统计呈现，参考 lemon-cleaner 交互范式。
- 默认勾选状态、危险项标记、reclaimable 总数展示，对齐常见清理软件认知。

### 设计素材边界

- `bg/`、`css3动效/`、`ui/` 为**设计素材参考**，不在此实现业务逻辑。
- 引用其视觉/动效思路时，提炼为 `reactbits` 或 `components/ui` 组件，不直接搬运整页 HTML。

### 代码风格

- TypeScript strict，**不**用 `any`（必要时用 `unknown` + 收窄）。
- 优先函数式与组合，避免过度 OOP。
- 注释用中文（与用户语言一致），但**不**写无信息量的注释。

---

## 6. 统计口径与双版本构建

### 统计口径（sizeMetric）

前后端通过 `sizeMetric` 参数贯通，用户可在界面切换后重新扫描。

| `sizeMetric` | 含义 | 对标 |
|---|---|---|
| **`logical`（默认）** | `metadata.len()` | Finder / Lemon 列表常见展示 |
| **`physical`** | Unix：`blocks * 512` 与 `len` 的 Mole 规则 | Mole「实际占用」 |

- 后端：`scan_home` / `scan_directory` 等命令接收 `sizeMetric` 参数，返回体中回填 `sizeMetric` 字段。
- 前端：扫描请求带 `sizeMetric`，结果展示与该值一致；切换口径需重新扫描。
- 默认展示 `logical`，与 Finder / Lemon 对齐；需要"磁盘真实占用"时切 `physical`，与 Mole 对齐。

### 双版本构建

| 版本 | profile | 沙箱 | 主扫描入口 |
|---|---|---|---|
| 官网完整版 | `full`（默认） | 通常关闭 App Sandbox | `scan_home`（`dirs::home_dir()`）一键扫描 |
| MAS 精简版 | `mas` | App Sandbox 开启 | `scan_home` + Security-Scoped Bookmark 引导授权 |

**核心原则**：

- **同一套 Rust 代码、同一 UI 入口**，差异仅在 entitlements 与构建 profile。
- MAS 版不把"每次手动选文件夹"作为主流程；扩大范围依赖 Security-Scoped Bookmark 首次引导授权。
- `tauri-plugin-dialog` 保留用于可选路径、调试、未来的书签/授权流，**不是**日常扫描唯一入口。

### 扫描入口与权限

- 主流程：`scan_home`（`dirs::home_dir()`）+ 前端「扫描我的 Mac」，**不要求用户每次**通过文件夹选择器选根目录。
- 高风险能力：不在沙箱应用内自动清理系统根路径、不直接卸载 `/Applications` 下其他应用。
- 清理走废纸篓（`trash` crate）与用户确认。

### 性能

- 扫描等重 I/O 在 Tauri 侧使用 `spawn_blocking`，避免阻塞 UI。
- 整盘扫描深层目录过慢时，引入 `rayon` / `jwalk` 并行遍历，目标秒级可感知响应（分阶段落地）。

---

## 7. 架构约束（待修订）

> ⚠️ 本节从 v1 架构约束文档继承。**部分内容作者认为不合适，计划后续人工修订。**  
> 在修订前，以下内容作为当前约束生效。

### 竞品对齐原则

| 维度 | 要求 |
|---|---|
| **交互与展示** | 以 **Lemon Cleaner** 为 UI/UX 基准：扫描入口、结果层级、勾选与统计呈现。 |
| **精度与深度** | 以 **Mole** 为算法参考：提供与 Mole 一致的物理口径能力；默认展示口径可与 Lemon / Finder 对齐。 |
| **数据契约** | 后端返回的 JSON 必须能映射到 Lemon 式结构：分类 → 子项 → 路径/文件列表。`ScanResult.rule_categories` 为编译期内嵌规则骨架，`items` 为整盘扫描树。 |

> ⚠️ 待修订点：作者认为"以 Lemon 为唯一 UI 基准"可能过窄，未来或综合 Lemon + CleanMyMac。

### 技术栈选择结论

坚持 **Rust 单栈**；不把 Mole 当作要嵌入的运行时代码，而把 Mole / Lemon 都当作需求与对照基准，在约束内重写。

> ⚠️ 待修订点：作者保留调整技术栈细节的权利（如是否引入部分原生能力）。

---

> **规则使用约定**  
> 1. 硬约束与红线（第 3 节）为最高优先级，任何内容不得与之冲突。  
> 2. 修订时直接编辑本文件对应章节即可。  
> 3. 所有规则用中文撰写，代码与命令保持英文。
