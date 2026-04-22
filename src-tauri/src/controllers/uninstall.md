# 卸载（Uninstall）功能 — 背景与问题交接文档

> 目的：让接手的 AI 快速理解「卸载功能目前做了什么、卡在哪个问题上」。
> 本文件只陈述背景、现状、问题与线索，不包含解决方案。

---

## 1. 项目背景

- **MoleStudio2** 是一款 macOS 清理/优化 GUI 软件。
- 技术栈：Tauri 2 + Rust（后端） + React 19 + TypeScript（前端）。
- 定位：**GUI 版的 Mole**（Mole = `tw93/mole`，一个 macOS 清理 CLI，Go + Bash 实现）。
- 卸载功能的目标：**界面数据与 Burrow 完全一致**。

> Burrow 是 Mole 的 Swift GUI 封装，它自己不扫描残留，而是调用 `mo uninstall --dry-run` 拿数据。
> 我们的后端是**用 Rust 重写了 Mole 的卸载逻辑**（不调用 `mo` 二进制），前端参考 Burrow 的交互。

参考项目路径：
- Mole（算法权威）：`/Users/liuy/mole_desktop/mvp/Mole`
- Burrow（GUI 封装，数据来源）：`/Users/liuy/mole_desktop/mvp/Burrow`
- MoleStudio2（本项目）：`/Users/liuy/mole_desktop/mvp/MoleStudio2`

---

## 2. 卸载功能架构

### 2.1 后端（Rust）

| 文件 | 职责 |
|---|---|
| `src-tauri/src/controllers/uninstall.rs` | Tauri command 入口（薄层） |
| `src-tauri/src/lib/uninstall/batch.rs` | 卸载核心逻辑 |
| `src-tauri/src/lib/uninstall/brew.rs` | Homebrew cask 检测/卸载 |
| `src-tauri/src/lib/core/app_protection.rs` | 残留发现 `find_app_files` |
| `src-tauri/src/lib/core/pkg_receipts.rs` | pkgutil receipt 扫描（非标准位置 app） |

后端三个 Tauri 命令：

| 命令 | 参数 | 说明 |
|---|---|---|
| `mole_list_apps` | 无 | 扫描已安装应用，返回清单 |
| `mole_uninstall` | `app_path: String, dry_run: bool` | 单个 app 的 dry-run 预览 / 执行 |
| `mole_uninstall_batch` | `app_paths: Vec<String>` | 批量卸载 |

### 2.2 前端（React/TS）

| 文件 | 职责 |
|---|---|
| `src/pages/Uninstall/index.tsx` | `ShellUninstall` 外壳：3 个 tab + 底部操作栏 |
| `src/pages/Uninstall/UninstallTab.tsx` | 卸载列表 + 行内展开残留预览 |
| `src/pages/Uninstall/mock.ts` | mock 数据（更新/启动项 tab 仍在用 mock） |

前端通过 `useTauri()` 自动生成的 api 调用后端（见 `src/hooks/useTauri.ts`）。

### 2.3 数据流

```
展开某个 app
  → 前端 UninstallTab.tsx 的 toggleExpand
  → tauri.mole_uninstall({ app_path, dry_run: true })
  → 后端 run_dry_run → collect_app_details
      → sibling guard（uninstall_live_bundle_has_other_install）
      → find_app_files 找残留
  → 返回 MoleUninstallResult（含 related_files 数组）
  → 前端 buildLeftovers 渲染残留列表
```

---

## 3. 已完成的功能

### 后端（对齐 Mole 逻辑）

- 应用清单扫描（app_dirs + pkg receipts + 去重 + 保护名单 + 背景应用过滤）
- 残留发现 `find_app_files`（bundle_id 精确路径 + 命名变体 + 版本后缀剥离 + bundle 边界匹配）
- 工具链只清缓存（DevEco/Android Studio/Xcode/Docker 等，不碰源码/SDK/密钥）
- 完整版 sibling guard（live 全盘扫描 + bundle_id 降级 + fingerprint 复查防 TOCTOU）
- brew cask nozap（sibling guard 时避免 `--zap` 删共享 config）
- force_kill_app 系统进程名守卫 + 优雅退出
- reverse-DNS 校验 + bundle_id 边界匹配
- 独立 CLI dotdir 跳过、嵌入 bundle id 扫描
- 元数据缓存（最小可行版，同步读写 + 7 天 TTL）
- 系统级文件 review-only（只展示不删除）
- `mole_list_apps` 返回 `source` / `uninstall_name` 字段

### 前端

- 卸载 tab 列表接入真实后端（`mole_list_apps`）
- 原生应用图标（iconService 预加载 + 色块兜底）
- 搜索 / 排序
- 展开残留异步填充（对齐 Burrow previewLoading）
- 卸载执行（`mole_uninstall_batch` + 确认弹窗 + 结果对话框）
- 底部操作栏（选中计数 + 大小统计）

---

## 4. 当前核心问题（未解决）

**现象**：

有一个测试 app 叫 `DBX.app`（`bundle_id = "com.dbx.app"`，路径 `/Applications/DBX.app`）。

- 列表能正确显示 DBX（`mole_list_apps` 正常返回）。
- 但**展开 DBX 时，前端残留只显示 "auto selected 1/1 app bundle"**，即只有 App Bundle 一项，没有任何残留文件。
- 而 Burrow 展开同一个 DBX，会显示多条残留，例如：
  - `support/com.dbx.app`
  - `cache/com.dbx.app`
  - `logs/com.dbx.app`
  - `webkit/com.dbx.app`

**期望**：展开 DBX 后，前端应该和 Burrow 一样列出这些残留文件。

---

## 5. 排查线索（时间线，含已排除的方向）

> 重要：以下记录了「试过什么、结果如何」，避免重复踩坑。

### 线索 A（已排除）：pkg receipt 扫描超时

- 最初怀疑 sibling guard 里的 `uninstall_live_bundle_has_other_install` 因 pkg receipt 扫描超时（8 秒预算）返回 `SCAN_PARTIAL`，导致残留被清空。
- 尝试：给 `pkg_receipts.rs` 加 1h TTL 磁盘缓存。
- 结果：**方向错了**，用户回退了该改动。根因不在 pkg receipt。

### 线索 B（已修复，但不是根因）：find_app_bundles 递归进入 .app 内部

- 发现 `find_app_bundles` 递归遍历时，会进入 `.app` bundle 内部，遇到 SIP/TCC 保护的不可读目录就标记 `indeterminate=true`，导致 live 扫描误判 `SCAN_PARTIAL`。
- 修复：发现 `.app` 后 `continue` 不递归进内部；`read_dir` 失败只在 root 层面标记 indeterminate。
- 结果：修复已应用，但**问题仍在**。

### 线索 C（关键）：展开 DBX 时，后端根本没收到 dry_run 请求

- 在 `collect_app_details`、`uninstall_live_bundle_has_other_install`、`find_app_files`、`run_dry_run` 里加了诊断日志（tag 见下）。
- 用户跑出来的现象：**终端只看到 `[uninstall.list_apps.entry]` 日志，完全看不到下面这些日志**：
  - `[uninstall.live_sibling]`
  - `[uninstall.sibling]`
  - `[uninstall.find_app_files]`
  - `[uninstall.dry_run]`

  这说明 **`mole_uninstall` 的 dry_run 请求根本没进到后端**。

### 线索 D（已修复，但用户反馈"还是没改对"）：前端 invoke 参数命名错误

- 发现前端调用参数用了 camelCase，但后端 `#[tauri::command(rename_all = "snake_case")]` 要求 snake_case。
- 本项目 Clean 页面验证了约定（snake_case）：
  - `tauri.clean_scan({ size_metric: 'logical' })`
  - `tauri.clean_apply({ args: { item_ids, scan_id } })`
- 修复了卸载页面的两处：
  - `UninstallTab.tsx`：`{ appPath, dryRun }` → `{ app_path, dry_run }`
  - `index.tsx`：`{ appPaths }` → `{ app_paths }`
- 结果：用户反馈「现在还是没有改对」。

---

## 6. 当前代码状态

### 6.1 关键契约（务必注意）

- 后端所有 Tauri 命令都标了 `#[tauri::command(rename_all = "snake_case")]`，**前端 invoke 参数必须用 snake_case**。
- 前端通过 `useTauri()` 自动生成 api，`invoke(cmd, payload)`，payload 的 key 是命令参数名。

### 6.2 已加的诊断日志（在后端 Rust，需要 `cargo build` 重新编译才生效）

| tag | 文件 | 记录内容 |
|---|---|---|
| `[uninstall.live_sibling]` | batch.rs | live 扫描 `result`（0/1/2/3）+ `indeterminate` |
| `[uninstall.sibling]` | batch.rs | sibling guard 判定：`live_rc` / `guard` / `discovery` / `bundle_after` |
| `[uninstall.find_app_files]` | app_protection.rs | 输入 `bundle` / `app` / `path` |
| `[uninstall.find_app_files]` | app_protection.rs | 返回 `result_count` + 完整 `paths` |
| `[uninstall.dry_run]` | controllers/uninstall.rs | `related_raw`（find_app_files 产出的原始路径） |
| `[uninstall.dry_run]` | controllers/uninstall.rs | `related_count`（size 过滤后的最终数量） |

### 6.3 关键代码位置

后端：
- `controllers/uninstall.rs`：`run_dry_run`（约 L614 起）、`mole_uninstall`（约 L605）、`mole_uninstall_batch`（约 L1182）
- `lib/uninstall/batch.rs`：`find_app_bundles`（约 L881）、`uninstall_live_bundle_has_other_install`（约 L950）、`collect_app_details`（约 L1224）
- `lib/core/app_protection.rs`：`find_app_files`（约 L1139）
- `lib/core/pkg_receipts.rs`：`scan_pkg_receipts_impl`

前端：
- `pages/Uninstall/index.tsx`：`handleUninstall`（约 L150 起）
- `pages/Uninstall/UninstallTab.tsx`：`toggleExpand`（约 L354 起）、`buildLeftovers`、`AppIcon`

> 行号可能因后续修改有偏移，以函数名 / tag 为准。

---

## 7. 待确认 / 下一步排查方向

以下问题尚未确定，需要拿到日志后判断：

1. **前端参数命名修复是否真正生效**：前端 TS 修改需要 Vite 热重载/重启；后端 Rust 日志需要 `cargo build` 重新编译。如果两者任一没生效，现象都不会变。
2. **展开 DBX 时，`mole_uninstall` 是否真的进入了后端**：如果还是只看到 `list_apps.entry`、看不到 `[uninstall.sibling]` 等日志，说明请求还没到后端（前端调用或 IPC 层问题）。
3. **如果请求进了后端**：看 `[uninstall.sibling]` 的 `guard` / `discovery` 字段：
   - `guard=guard_login, discovery=""` → sibling guard 清空了残留（继续看 `[uninstall.live_sibling]` 的 `result` / `indeterminate`）
   - `guard=none, discovery="DBX"` → sibling guard 正常，问题在 `find_app_files`（看它的 `result_count`）
4. **如果 `find_app_files` 返回空**：看输入 `bundle` / `app` 是否正确（`com.dbx.app` / `DBX`）。
5. **如果 `find_app_files` 返回了路径，但前端仍空**：看 `[uninstall.dry_run]` 的 `related_raw` vs `related_count`，判断是否被 `size == 0` 过滤。

---

## 8. 补充说明

- 更新 tab（`UpdatesTab.tsx`）和启动项 tab（`StartupTab.tsx`）**仍是 mock 数据**，未接后端，这与当前 DBX 残留问题无关。
- 卸载 tab 的残留 item 级勾选目前是「摆设」：展开后能勾选残留，但卸载走的是整 app 卸载，勾选的残留不影响实际删除。

---

## 9. 根因诊断（2026-08-16 已定位，实锤）

> 诊断方法：新增 `src-tauri/tests/dbx_probe.rs` 探针，绕开 IPC/前端直接调
> `collect_app_details(["/Applications/DBX.app"])` 并挂 stderr logger 复现全链路。
> 复现命令：`cargo test --test dbx_probe -- --nocapture`。

### 9.1 根因：`run_with_timeout_capture` 管道死锁

`lib/core/timeout.rs` 的 Rust fallback 分支（无 gtimeout/timeout 的机器上走这里）：
`stdout(Stdio::piped())` 后**先 `wait_timeout` 等子进程退出、再读管道**。
macOS 管道缓冲约 64KB，子进程输出超过该值就会写阻塞，父进程又在等退出 → 死锁 →
撑满超时后 SIGKILL → 返回 `None`。

本机实测：`pkgutil --files org.golang.go`（16093 行 ≈ 800KB）→ 5.3s 后返回 `None`；
`pkgutil --files org.nodejs.npm.pkg`（2174 行）同理。`pkgutil --pkgs`（41 行小输出）正常。

### 9.2 完整因果链

1. `scan_pkg_receipts_impl`（pkg_receipts.rs）：2 个大输出 pkg 各烧 5s 死锁 ≈ 10s，
   超过 8s 总预算 → 返回 `complete=false`（线索 A 方向其实对了，只是根因不是慢而是死锁，
   加缓存治标所以被误判为「方向错了」）。
2. `uninstall_live_bundle_has_other_install`（batch.rs）：`pkg_complete=false`
   → `scan_indeterminate=true` → 实际没扫到兄弟（result=1）也被改成 `result=3`（SCAN_PARTIAL）。
3. `collect_app_details`：SCAN_PARTIAL 按「有兄弟」保守处理 → `guard=guard_login`，
   `discovery_app_name=""`、`bundle_id="unknown"` → **跳过 `find_app_files`** →
   `related_files` 为空 → 前端只剩 App Bundle。
4. 实测日志：`live_sibling result=3 indeterminate=true` →
   `sibling guard=guard_login discovery="" bundle_after=unknown` → `related_cnt=0`。

### 9.3 为什么线索 C 误判「请求没进后端」

- 这条链单次执行约 **22 秒**（inventory + live 各一次 pkg 死锁扫描 ≈ 10s×2，再加全盘
  walk + 每个 app 一次 plutil），期间终端无任何日志（第一条日志在 live 扫描结束后才打），
  短时间内观察会误以为请求没到。
- 前端 `toggleExpand` 的 `.catch(() => {})` 吞错，且 `detailCache` 缓存了
  `preview: null` 后**折叠再展开不会重试**，UI 永远停在「1/1 app bundle」。

### 9.4 修复记录（2026-08-16 已实施，探针验证通过）

1. **已修**：timeout.rs 管道死锁——`run_with_timeout_capture` 与
   `run_with_timeout_capture_lossy` 的 Rust fallback 分支，spawn 后立即把 stdout
   交给 reader 线程 `read_to_end` 并发排空，`wait_timeout` 等待期间不再持有读端。
   修复后 `pkgutil --files org.golang.go` 从 5.3s→None 变为 124ms 返回 824KB。
2. **已修（第二根因，9.1 之外新发现）**：`installed_app_inventory`（batch.rs）的
   /Volumes/*/Applications 缺少 SH bin/uninstall.sh L377-381 的 `-ef` 去重。
   镜像目录（`/Volumes/Macintosh HD/Applications` firmlink、`/Volumes/DBX/Applications`
   DMG symlink）里的同 bundle app 被 `has_surviving_sibling` 当「幸存兄弟」→ 名字碰撞
   → `guard=guard_login` → 残留发现被跳过。修复：新增 `dirs_same_file`（stat dev+ino，
   跟随符号链接）跳过镜像目录，与 mole_list_apps 的 same_file 去重对齐。
   （live 扫描之所以不受影响：`live_candidate_is_selected` 本身按 inode 排除镜像。）
3. **已修（前端）**：UninstallTab.tsx 展开失败时标记 `failed: true` 并 console.error 留
   日志，折叠再展开会重试（之前缓存 preview:null 永不重试）。
4. 探针结果：`guard=none discovery="DBX" bundle_after=com.dbx.app`，
   `find_app_files result_count=4`（Application Support / Caches / Logs / WebKit 的
   com.dbx.app），与 Burrow 完全一致；单次耗时 22s → 1.1s。
5. **不要急着改**的次要分歧：`find_app_bundles` 不把 root 自身当候选，/Volumes 顶层
   `.app`（如本机挂载的 `/Volumes/DBX/DBX.app`，同 bundle id 真兄弟）永远漏检；
   但 SH 靠 `find <root>` 会吐出 root 自身 + 另有 same-basename mirror 去重兜底。
   动这里反而会引入新的不一致。

> 注：`cargo test --lib` 有 4 个预存环境失败（analyze/delete 沙箱保护、diagnostics
> 挂载卷计数、cache 磁盘缓存），在改动前的代码上同样失败，与本次修复无关。

---

## 10. 第二轮修复（2026-08-16）：前端只见 Caches/WebKit、缺 Logs/Application Support

> 后端探针已返回 4 条，但前端只渲染 2 条 → 问题在 controllers/uninstall.rs 的
> `run_dry_run` 尺寸预处理与过滤，两个独立 bug 各吞一条。

### 10.1 根因

1. **du 输出按空白拆词，含空格路径被截断**（吞掉 Application Support）：
   `batch_du_sizes_multi` 用 `split_whitespace()` 取前两个 token，
   `"172<TAB>/Users/liuy/Library/Application Support/com.dbx.app"` 被拆成
   `["172", "/Users/liuy/Library/Application", "Support/com.dbx.app"]`，
   path 截断为 `/Users/liuy/Library/Application`（幽灵 key）→ 真实路径查 size
   map 得 None → 跳过。所有含空格目录（Saved Application State、Application
   Scripts、Group Containers 等）同病。
2. **`size == 0` 过滤吞掉空目录**（吞掉 Logs）：`~/Library/Logs/com.dbx.app`
   只有一个 0 字节 DBX.log，`du -skP` 报 0 → `run_dry_run` 的
   `if size == 0 { continue; }` 直接跳过。Burrow（mo dry-run）不按 size 过滤。

### 10.2 修复

1. `batch_du_sizes_multi`：改为按**第一个空白处** `split_once` 拆一次，左边 size、
   右边整体是路径（trim）。
2. `run_dry_run`：related_files 与 review_only_files 两处都去掉 `size == 0` 过滤，
   只保留「size map 无 key → 路径不存在/不可读」的跳过；前端对 size=0 渲染 "—"。
3. 新增端到端探针 `probe_dbx_dry_run_related_files`（tests/dbx_probe.rs）：直接调
   `mole_uninstall(dry_run=true)` 断言 4 条残留全部返回（含空格路径与 0 字节 Logs）。
   实测：Application Support=176128B、Caches=8192B、Logs=0B、WebKit=851968B，
   与 Burrow 完全一致。

## 11. 第三轮修复（2026-08-16）：find_app_files 与 SH 的对齐缺口（bundle_leaf 等 9 组）

> 第三方分析（/Users/liuy/mole_desktop/mvp/2）指出 find_app_files 缺 5 个部分，
> 逐条验证属实后，连同自查发现的 4 组小 gap 一并补齐（app_protection.rs）。

### 11.1 补齐清单（均对齐 Mole lib/core/app_protection.sh）

1. **bundle_leaf 推导**（SH 第 1018-1058 行）：leaf 必须扩展 display name 本身
   （反向 DNS 有效、leaf≥8 字符、含驼峰、小写 leaf 以小写无空格名开头且更长、
   余下以大写/数字开头），产出 leaf 原样 + `"{app_name} {rest_spaced}"` 两个变体。
   sed 空格插入逻辑先与 macOS BSD sed 逐例对拍（"XMLParser"→"XML Parser"、
   "Desktop" 不变），抽为纯函数 `bundle_leaf_variants` + `spread_camel_spaces`。
2. **CrashReporter plist 通配符**（SH 第 1573-1588 行）：`maxdepth 1 -type f`，
   收 `{app_name}_*.plist` 与 `{nospace_name}_*.plist`（nospace≥3 才扫）；
   删除了原来多余的精确目录候选（SH 无此条目）。
3. **VSCode 分支**（SH 第 1473-1489 行）：命中条件改为 `microsoft.*[vV][sS][cC]ode`
   （`is_vscode_bundle_id`）；ShipIt 两个都收；Insiders 只收 `.vscode-insiders` /
   `Application Support/Code - Insiders` / `Caches/…VSCodeInsiders`，稳定版只收
   `.vscode` / `Application Support/Code` / `Caches/com.microsoft.VSCode`。
4. **Anki**（SH 第 1505-1510 行）：collect_toolchain 下 bundle==net.ankiweb.anki
   或 app_name==Anki → 收 `Application Support/AnkiProgramFiles`。
5. **Raycast 扩展**（SH 第 1512-1571 行）：所有 `*raycast*` 扫描排除 raycast-x
   （v2 独立 app）；补 `-type d` 限定；新增 Caches `maxdepth 2` 扫描与
   `Application Support/Code/User/globalStorage` 扫描。
6. **`.cache` 系列**：主块 `.cache/{lowercase_name}`（SH 第 989 行）；空格变体
   `.cache/{lowercase_nospace|hyphen|underscore}`（SH 第 1081-1083 行）；
   base_name `.cache/{base_lowercase}`（SH 第 1100 行）。
7. **Preferences 无 .plist 形式**：主块 `Preferences/{app_name}`（SH 第 968 行）；
   空格变体 `Preferences/{nospace|underscore|hyphen}` 及对应 `.plist`（SH 第
   1068-1076 行）；base_name `Preferences/{base}` + `.plist`（SH 第 1096-1097 行）。
8. **Saved Application State**：主块 `{app_name}.savedState`（SH 第 970 行）；
   空格变体 `{nospace}.savedState`（SH 第 1070 行）；base_name（SH 第 1098 行）。
9. **两处小语义差异**：app_name 候选门槛 `!empty` → `≥2`（SH 第 963 行）；
   base_name 候选补 `len>2` 条件（SH 第 1091 行）；Caches/Logs nospace 变体移入
   空格变体块（SH 把它们放在第 1062-1087 行块内）。

### 11.2 验证

- 单测：`spread_camel_spaces_matches_bsd_sed`（与 BSD sed 对拍）、
  `vscode_bundle_id_match_follows_sh_regex`、`bundle_leaf_variants_match_sh_examples`
  （AyuGram 正例 + GoogleChrome/64Gram/短 leaf 反例）全过。
- 探针：`probe_vscode_find_app_files`（dbx_probe.rs）对真实 VSCode 断言
  `~/.vscode`、`Application Support/Code`、`Caches/com.microsoft.VSCode` 被收集，
  且 CrashReporter 扫描不误收 Electron_* 等无关 plist；实测 7 条全部符合预期。
- 回归：dbx_probe 5 个探针全过；`cargo test --lib` 118 通过，仅剩 4 个
  沙箱环境预存失败（与修改前一致）。

## 12. 第四轮修复（2026-08-16）：三个安全防护级差异（app-path 扫描 / mutable ancestor / identity 绑定）

> 全链路审计确认主卸载流程已与 Mole 对齐后，仍存在 3 个「安全防护级」差异。
> 均为 SH 端在异常路径（bundle id 降级、root-owned app、预览后被替换）下
> 的防护，缺失时会导致 Rust 端比 Mole 更激进。本轮补齐。

### 12.1 差异一：stop_launch_services 缺 app_path 扫描（batch.sh L393-457）

- SH 语义：bundle_id 降级 unknown 时，仍必须扫描 ProgramArguments 引用 app
  路径的 plist（`grep -qF` 字节匹配后 unload，**不删除**，删除归 remove_file_list）。
- Rust 补齐：`stop_launch_services` 加 `app_path` 参数；新增
  `unload_launch_plists_matching_app_path`（user 域 + has_system_files 时系统域，
  maxdepth 1 扫 *.plist → 读内容 → 命中则 unload）与
  `plist_references_app_path`（`content.windows(needle.len()).any()`，grep -qF 等价）。

### 12.2 差异二：_mole_privileged_path_has_mutable_ancestor 缺失（file_ops.sh L1163-1215）

- SH 语义：逐级父目录检查符号链接 / 非 root 属主 / 022 写位 / ACL 写权限 /
  不可读元数据，任一命中判 mutable（fail-closed）；preflight 在 batch.sh L1518-1527
  （needs_sudo && !brew_cask 时拒绝转 manual_removal）；mole_delete 内部也查，
  返回 MOLE_ERR_MUTABLE_PARENT=15。
- Rust 补齐（file_ops.rs）：`_mole_privileged_path_has_mutable_ancestor` 完整翻译
  （symlink / uid≠0 / mode&0o022≠0 → true；EUID==invoking_uid 走 access W_OK；
  EUID==0 走 `sudo -n -u #uid /bin/test -w`；其余 fail-closed true）；
  `mole_delete` 加 `expected_identity` 参数并在 validate 后插入 mutable 检查
  （返回 15）与 identity 检查；`diagnose_removal_failure` 加 15 码文案。
- **关键实测**：本机 /Applications = root:admin 775（group-write 位）→ SH 判
  MUTABLE。即 SH 端 permanent 模式下 /Applications 里 root-owned 的 app 会被
  preflight 拒绝转 manual removal——这是 Mole 的真实行为，Rust 补齐后一致。

### 12.3 差异三：app identity（inode）绑定缺失（batch.sh L1269-1314）

- SH 语义：`_batch_selected_app_identity` = `stat -f%d:%i:%m` → `dev:ino:mode`
  （mode 是八进制权限位字符串，如 755）；Info.plist 另有 info identity（缺失
  为 "missing"）；执行期 4 个 `_batch_selected_app_plan_matches` 复查点
  （SH L1869/L1936/L2006/L2047），失败 reason="selected app changed after preview"；
  mole_delete 第 3 参数 expected_identity（重查拒绝）。
- Rust 补齐（file_ops.rs + batch.rs）：`stat_path_identity` 用 `{:o}` 八进制
  格式化（对齐 SH %m，避免输出 493 而非 755）；AppDetail 加
  `expected_app_identity` / `expected_info_identity` / `manual_removal_reason`；
  collect 时记录 identity（Info.plist 缺失 → manual_removal）；uninstall_one_app
  四个检查点（pre-fingerprint / pre-teardown / pre-delete / brew 兜底前）；
  所有 app bundle 相关的 mole_delete 调用传 Some(identity)。

### 12.4 前端适配（3 处）

1. `src/types/mole.ts`：MoleUninstallResult 加 `manual_removal?: boolean; reason?: string`。
2. `src/pages/Uninstall/UninstallTab.tsx`：LeftoverPanel 加 manual_removal 黄色警告块。
3. `src/pages/Uninstall/index.tsx`：UninstallBatchResult 加 `manual_removal_apps?: string[]`，
   结果弹窗加「无法安全卸载（请在 Finder 中手动移到废纸篓）」warning 列表。

### 12.5 验证

- 单测（batch.rs tests）：`plist_app_path_reference_detection_matches_grep_f`、
  `selected_app_plan_matches_binds_preview_identity`（inode 变化拒绝 / 空 expected 拒绝）、
  `stat_identity_format_matches_sh_stat_permissions` —— 9/9 全过。
- 探针：dbx_probe 新增 `probe_mutable_ancestor_matches_sh`（/Applications 判
  mutable、不存在路径 fail-closed、identity 三段八进制）；实测
  `root identity=16777233:2:755` 与 SH `stat -f%d:%i:%m` 格式一致；6 探针全过。
- `cargo check --lib / --tests` 通过（仅 4 个与本次无关的既有警告）。

### 12.6 附加发现：后台项残留检测用了 sfltool dumpbtm（与 SH 相反）

- 最终函数级比对扫描发现：Rust `check_btm_leftovers` 用 `sfltool dumpbtm` 检测
  后台项残留；SH 权威实现 `_uninstall_match_loaded_background_items`
  （batch.sh L145-191）明确注释「Deliberately does NOT consult sfltool dumpbtm:
  非特权 dumpbtm 每次批处理都会弹 "sfltool wants to make changes"
  管理员密码对话框，且 registered-but-unloaded 的 BTM 记录本就是 macOS 下次登录
  时清理的设计残留」。
- 修复：重写为 SH 方案——对每个成功卸载 app 的 bundle_id + login_item_helpers
  逐 label 校验 reverse-DNS 后 `launchctl print gui/$uid/$label` 探测，仍 loaded
  即加入「后台项残留」warning；TEST_MODE/TEST_NO_AUTH 时安静跳过。
- 验证：新单测 `btm_leftovers_quiet_in_test_mode_and_skips_unmatched_paths`；
  uninstall 模块 14 个单测全过，6 探针全过。

### 12.7 函数级 SH↔Rust 比对结论（其余均等价）

- `decode_bundle_id_list` → Rust 直接枚举 LoginItems 目录读 Info.plist，不经
  base64 管道（实现路径不同、语义等价）。
- `_batch_refresh_selected_app_bundle_id` → collect 内 read_bundle_identifier。
- `_uninstall_insert_sorted_live_record` → live_records.sort() + dedup()。
- `_uninstall_materialize_complete_pkg_apps` → pkg_receipt_nonstandard_app_paths_complete()。
- `append_line` / `format_uninstall_preview_path` → 纯 TUI 辅助，无需翻译。
