# Analyze 后端数据层升级与 Mole 对齐说明

> 本文档总结 Analyze（磁盘分析）模块后端数据层升级工作，记录与 Mole 最新代码的对齐决策、修复的问题与遗留事项。

## 1. 背景

| 事实 | 说明 |
|---|---|
| V1 Rust 引擎最后同步 Mole | 2026-06-26（b92b4b1 "升级mole"） |
| Mole 最新数据层提交 | 2026-08-09 之后仅剩 TUI bar 渲染微调，无数据层变更 |
| 差距 | 约 6 周、24 个提交，其中 5 组数据层差异需要对齐 |

策略：**以 V1 GUI 为基础，引擎核心逻辑对齐 Mole 最新代码**（不 100% 照搬——Mole 含 TUI/CLI 专属代码，V2 只移植 GUI 相关的数据层语义）。

## 2. 工作阶段

| 阶段 | 内容 | 状态 |
|---|---|---|
| Phase A | 前端接线：真实 IPC 接入（替换 mock） | ✅ |
| Phase B | 引擎安全对齐：EDR 保护 / trash(8) / Parallels + schema v3 | ✅ |
| Phase C | 引擎性能对齐：删除计数 count=1 / overview 快照有界 | ✅ |
| Phase D | 技术债清理（见 §7 遗留事项） | 待定 |

## 3. Mole 最新代码 5 组差异对齐明细

### 3.1 EDR 缓存保护（Mole f0426010）

- 问题：Mole 新增 Endpoint Security 缓存保护，不依赖 `HOME` 环境变量；V1 缺失。
- Rust 落点：`src-tauri/src/cmd/analyze/delete.rs`
  - `is_protected_analyze_delete_path`：EDR 检查置于保护链**最前**（即使 `env -u HOME mo analyze` 也不得漏过 Falcon 缓存）。
  - `ENDPOINT_SECURITY_BUNDLE_PREFIXES`：9 个厂商 bundle 前缀（crowdstrike / sentinelone / eset / jamf / paloaltonetworks / cisco…）。
  - `is_endpoint_security_cache_path`：匹配 `/private/var/folders/` 或 `/var/folders/` 下的厂商缓存路径（大小写不敏感）。

### 3.2 trash(8) 优先（Mole 8cd82fe8，issue #474 SSH 场景）

- 问题：SSH 会话中 Finder 不可用，`trash` crate 会失败；Mole 改为 `/usr/bin/trash` 优先。
- Rust 落点：`src-tauri/src/cmd/analyze/delete.rs`
  - `TRASH_BINARY = "/usr/bin/trash"`，绝对路径调用、不传 `"--"`。
  - `move_to_trash_via_binary`：`ChildExt::wait_timeout(30s)`，超时 kill 后 fallback。
  - `move_to_trash` 链：binary 成功即返回；失败 fallback `trash::delete(&abs)`（crate 3.x）。

### 3.3 Parallels 移出 skip 表 + 缓存 schema v3（Mole 668fbe4b）

- Rust 落点：
  - `src-tauri/src/cmd/analyze/constants.rs`：`default_skip_dirs` 删除 `"Parallels"`（普通 Parallels VM 存储不再按名跳过，纳入扫描）。
  - `src-tauri/src/cmd/analyze/cache.rs`：`CACHE_SCHEMA_VERSION: u32 = 3`。

### 3.4 删除计数恒为 1（Mole 9cb63949）

- 问题：删除前递归 `WalkDir` 数文件会让大目录在移动开始前显得假死。
- Rust 落点：`src-tauri/src/cmd/analyze/delete.rs`
  - 只做 `fs::symlink_metadata` 存在性检查（对齐 Go `os.Lstat`，兼容 broken symlink），计数恒为 `1`。

### 3.5 overview 快照有界（Mole 7cf9e382）

- 问题：overview 大小快照 JSON 无限增长。
- Rust 落点：`src-tauri/src/cmd/analyze/constants.rs` + `cache.rs`
  - `overview_cache_max_entries = 1000` / `overview_cache_keep_entries = 900`。
  - `evict_overview_snapshots`：超过 1000 条按 `updated` 排序删到 900。
  - `refresh_divisor = 8`：相同 size 且 `updated` 距今 < TTL/8 时跳过重写（no-op）。
  - `OverviewSizeSnapshot` 增加 `schema_version` 字段（`#[serde(default)]` 向后兼容）。
  - 加载时清理：schema 不匹配 / size ≤ 0 / 过期的快照直接丢弃。

## 4. 过程中发现并修复的既有 Rust 偏离 Go 的 Bug

| Bug | 根因 | 修复 |
|---|---|---|
| tempdir 被误判为受保护路径 | Rust `SYSTEM_PREFIXES` 含 `"/private/"` 整树前缀，拦截了 `/private/var/folders/...` 下的临时目录 | 对齐 Go `isCriticalAnalyzeDeletePath`：`/private`、`/opt` 只精确保护根本身 + 特定系统子树；`/private/var/folders` 子树、`/private/var/log`、`/opt/local` 不拦截（`protected.rs`） |
| 缓存冷热测试失败 | Rust `load_cache_from_disk` 拒绝 `needs_refresh=true` 缓存，而冷启动写入时恰标记 true，导致 warm 永远 miss | 对齐 Go `loadCacheFromDisk`：subdir 缓存命中不做新鲜度校验（`scanner.rs` 改用 `load_raw_cache_from_disk`），新鲜度由删除后 `invalidate_cache` 保证 |

## 5. 前端接线契约（本目录下的真实 IPC）

| 场景 | 调用 | 参数形状 |
|---|---|---|
| 浏览扫描 | `tauri.mole_analyze({ path, overview: false, skip_cache })` | `path: string` |
| overview | `tauri.mole_analyze({ path: '', overview: true })` | 空 path ⇒ 全机 overview |
| 删除（废纸篓） | `tauri.mole_analyze_trash({ args: { paths } })` | 注意 `args` 包裹层 |
| 进度 | `EVT_ANALYZE_SCAN_PROGRESS`（`useAnalyzeData` 内 `.listenIpc`） | 逐目录推进 |

类型契约注意：Rust `JsonEntry` 的 `insight / cleanable / protected` 为 `skip_serializing_if = is_false`，`false` 时字段缺失；消费侧一律 `?? false` 归一化。

## 6. 验证

- V1（MoleStudio）与 V2（MoleStudio2）引擎 `cargo test` 各 **87 全过**。
- 双仓库同步：`diff` 验证 V2 引擎文件与 V1 HEAD 一致后 `cp` 同步 5 个文件：
  `delete.rs` / `protected.rs` / `constants.rs` / `cache.rs` / `scanner.rs`。

## 7. 遗留事项（Phase D，不影响逻辑闭环）

1. **文件打开错误提示未恢复**：`contexts/AnalyzeContext.tsx` 的 `onActivate` 仍为静态阶段静默处理（`openPath(...).catch(() => {})`），注释标记"接后端后恢复错误提示"。
2. **图标预加载暂缓**：`hooks/useAnalyzeData.ts` 中 `iconService` 预加载因 `BATCH_IDLE` 类型问题未接入，当前 emoji 降级展示。
3. **tsc 7 个既有告警**：未使用变量（`accentColor`、`three`、`CircleButton` 等），不影响构建。
