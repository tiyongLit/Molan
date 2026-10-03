# Optimize 模块续开发指南

> 本文档是 V2 Optimize 模块的**交接快照**。目的：隔一段时间回来后，直接读本文档即可继续开发，无需重新梳理历史。
> 最后更新：2026-08-17（后端六态化改造完成，前端处于静态 mock 阶段）。

---

## 1. 一句话现状

- **后端（Rust）**：✅ 已全部完成并与 Mole 权威实现对齐——21 个任务六态化、任务目录对齐、8 个加固提交同步、`cargo check` 通过、140 个测试全通过。
- **前端（React）**：⏳ 静态 UI 已完成，数据来自 `mock.ts`，**尚未接线后端**。类型已先行同步到 `src/types/mole.ts`。
- **下一步**：前端接线（见 §5 路线图），后端只需维护性对齐（见 §6 守则）。

---

## 2. 后端现状速查

### 2.1 关键文件

| 文件 | 职责 |
|---|---|
| `src-tauri/src/lib/optimize/outcome.rs` | 六态枚举 `OptimizeOutcome` + `from_counts` 折算（含 2 个测试） |
| `src-tauri/src/lib/optimize/tasks.rs` | 21 个任务实现 + `execute_optimization(action)` 分派 + 辅助函数 |
| `src-tauri/src/lib/optimize/diagnostics.rs` | 性能诊断采样（含 #960 `extract_mount` 修复 + 测试） |
| `src-tauri/src/lib/optimize/maintenance.rs` | `fix_broken_preferences`（plist 批量 lint 修复） |
| `src-tauri/src/lib/check/health_json.rs` | 任务清单 `ROWS`（21 行，dry_run 的 tasks 来源） |
| `src-tauri/src/controllers/optimize.rs` | `mole_optimize` 命令 + `optimize::progress` 进度事件 + 结果 JSON |
| `src-tauri/src/lib/core/timeout.rs` | 超时工具三件套（见 2.3） |
| `src-tauri/src/lib/core/bundle_resolver.rs` | `bundle_has_installed_app`（bool）+ `_checked`（三态 fail-closed） |

### 2.2 六态协议（#1293）

```rust
pub enum OptimizeOutcome { Applied, Unchanged, Skipped, Unavailable, Attention, Failed }
```

| 状态 | 含义 |
|---|---|
| `applied` | 变更已完成（dry-run 下为"将完成"） |
| `unchanged` | 检查完成，无需变更 |
| `skipped` | 策略/上下文主动阻止（白名单、无 sudo、VPN、电池、env 未启用） |
| `unavailable` | 主机缺能力（sqlite3/pgrep/periodic/lsregister 缺失、通知库无路径） |
| `attention` | 检查完成，发现需用户处理的问题（disk_verify 发现错误、login_items 有 broken） |
| `failed` | 本可执行但未完成（超时、信号、命令失败、探测异常） |

**折算规则 `from_counts(applied, failed, skipped)`**：`failed>0 → failed`；`applied>0 → applied`；`skipped>0 → skipped`；否则 `unchanged`。（与 Mole `optimize_task_result_from_counts` 一致，有测试锁定）

### 2.3 超时工具三件套（timeout.rs）

| 函数 | 返回 | 用途 |
|---|---|---|
| `run_with_timeout(sec, cmd, args)` | `i32` 退出码（124=超时） | 不关心 stdout |
| `run_with_timeout_capture(sec, cmd, args)` | `Option<String>`（None=失败/超时） | 常规捕获 |
| `run_with_timeout_capture_rc(sec, cmd, args)` | `(i32, Option<String>)` | **区分"正常失败/超时(124)/信号(≥128)"的 fail-closed 场景** |

### 2.4 21 任务清单（三处必须一致）

`execute_optimization` 分派 / `health_json::ROWS` / Mole `catalog.sh` 注册——顺序与 action 逐一相同：

```
system_maintenance, cache_refresh, saved_state_cleanup, fix_broken_configs,
network_optimization, sqlite_vacuum, launch_services_rebuild, prevent_network_dsstore,
legacy_overrides_audit, network_stack_optimize, disk_permissions_repair,
spotlight_index_optimize, spotlight_orphan_rules_cleanup, periodic_maintenance,
shared_file_list_repair, disk_verify, login_items_audit, quarantine_cleanup,
launch_agents_cleanup, notification_cleanup, coreduet_cleanup
```

> 已删除（Mole 同步移除）：`font_cache_rebuild`、`bluetooth_reset`、`dock_refresh`、`memory_pressure_relief`。

### 2.5 控制器协议（controllers/optimize.rs）

**命令**：`mole_optimize` —— `#[tauri::command(rename_all = "snake_case")]`，参数扁平：
```rust
pub async fn mole_optimize(app, dry_run: bool, selected_actions: Option<Vec<String>>) -> Result<Value, String>
```

**进度事件** `optimize::progress`（serde tag = `phase`）：

| phase | 关键字段 |
|---|---|
| `begin` | `total`, `actions` |
| `task_start` | `index, total, action, name, description` |
| `task_skipped` | `index, total, action, name, reason`（白名单跳过，不走 handler） |
| `task_done` | `index, total, action, name, ok, outcome(六态), duration_ms, error?` |
| `complete` | `total, success, failed, skipped, duration_ms, outcomes(六态统计对象)` |

**执行细节**：
- execute 前 `ensure_admin_session()` 并设 `MOLE_OPTIMIZE_SUDO_AVAILABLE` 环境变量；任务内用 `optimize_sudo_available()` 判断（测试模式 `MOLE_TEST_MODE`/`MOLE_TEST_NO_AUTH` 硬拒绝）。
- 每个任务包 `catch_unwind`：panic → 折算 `failed`；未知 action → `failed("unknown action")`。
- 白名单：`is_whitelisted_optimize` 命中 → `task_skipped` 事件 + 结果 `status="skipped"`。
- execute 开始前 `remove_var("MOLE_DRY_RUN")`，确保 lib 不误读 dry-run。

**结果 JSON**（dry_run 与 execute 两种形态，前端类型已对齐 `MoleOptimizeResult`）：
- dry_run：`mode/tasks/diagnostics/system_info/summary{total_tasks,safe_count,would_apply_count}`
- execute：`mode/results[{task_id,task_name,status,error,duration_seconds}]/system_info/stats{...}/summary{total_tasks,applied_count,failed_count,skipped_count,outcomes,duration_seconds}`

### 2.6 关键环境变量

| 变量 | 谁设/谁读 | 含义 |
|---|---|---|
| `MOLE_OPTIMIZE_SUDO_AVAILABLE` | 控制器设，任务读 | sudo 会话可用性 |
| `MOLE_ASSUME_VPN_ACTIVE` | 外部可设 | VPN 检测覆盖（1/true/yes → 有；0/false/no → 无） |
| `MOLE_ENABLE_DISK_VERIFY` | 外部可设 | disk_verify 总开关（≠1 → skipped） |
| `MOLE_DRY_RUN` | lib 读 | dry-run 模式 |
| `MOLE_OPTIMIZE_SPOTLIGHT_SLOW_SEC` | spotlight 任务读 | 慢速阈值 |
| `MOLE_SQLITE_MAX_SIZE` | sqlite 任务读 | 100MB 跳过上限 |
| `OPTIMIZE_CACHE_CLEANED_KB` 等 | 任务写，控制器收集 | stats 统计 |

---

## 3. 前端现状速查

| 文件 | 状态 |
|---|---|
| `src/types/mole.ts` | ✅ 已同步：`OptimizeOutcomeStatus`、`OptimizeProgressEvent`（task_done 带 `outcome`、complete 带 `outcomes`）、`MoleOptimizeSummary.outcomes?` |
| `src/pages/Optimize/index.tsx` | 静态 UI + mock 调用点（L180 立即分析、L256 开始优化，注释标明接后端位置） |
| `src/pages/Optimize/mock.ts` | mock 数据（接后端后删除） |
| `src/pages/Optimize/components/` | `OptimizeLoading` / `OptimizeLogPanel` / `OptimizeStatusBar` / `OptimizeResult`（+ 对应 css） |
| `src/hooks/useTauri.ts` | 统一 IPC 入口：命令已注册（`mole_optimize` 在 `constants/tauri-commands.ts`），事件用 `onIpcEvent` 订阅 |
| `src/constants/tauri-events.ts` | `EVT_OPTIMIZE_PROGRESS = 'optimize::progress'` |

---

## 4. 已解决的坑（别踩回去）

1. **#960**：`extract_mount_from_line` 仅 `/dev/disk*` 行是真实挂载点，`image-alias`/`icon-path`/`shadow-path` 行含绝对路径但不是挂载点。
2. **#1367**：sqlite `policy_skipped_paths` 非空时头条不得宣称 "already optimized"；pgrep 退出码非 0 非 1 → 任务 failed。
3. **#1368**：通知库路径解析 group container（macOS 15+）优先，`getconf DARWIN_USER_DIR` 兜底；无路径 → `unavailable` 而非 unchanged。
4. **fail-closed 三态**（SH 26f4d47a 对齐）：探测异常（超时 124 / 信号 ≥128）→ 任务 failed，**不得**静默继续或误删：
   - `bundle_has_installed_app_checked` → `None`（orphan rules 用）
   - `plist_get_program` → `Err(())`（launch_agents 用）
   - `run_launchctl_unload` → `false`（5s 超时传播）
   - `has_active_vpn_interface` → `None`（network_stack 用）
5. **白名单双层**：控制器层 `is_whitelisted_optimize`（整任务跳过）+ 任务内 `is_path_whitelisted_from_global`（单路径跳过）。
6. **健康快照双读**：execute 完成后 `collect_health_json()` 再跑一次拿 system_info，与 dry_run 一致。

---

## 5. 下一步路线图（前端接线）

### Step 1 — dry_run 接线
`index.tsx` L180 附近：把 `OPTIMIZE_MOCK_DRY_RUN` 换成真实调用：

```ts
import useTauri from '@/hooks/useTauri'
import type { MoleOptimizeResult } from '@/types/mole'

// useTauri 命令由 constants/tauri-commands 生成，参数扁平透传
const result: MoleOptimizeResult = await useTauri().mole_optimize({ dry_run: true })
```

返回的 `tasks[].status` 此时为 `"pending"`；渲染 `diagnostics`（瓶颈提示条）与 `summary.safe_count`。

### Step 2 — 进度事件订阅
在组件 `useEffect` 里订阅（参考 `useTauri` 的 `onIpcEvent` 用法，cleanup 用 `AbortController`）：

```ts
useTauri().onIpcEvent<OptimizeProgressEvent>(EVT_OPTIMIZE_PROGRESS, (payload) => {
  switch (payload.phase) {
    case 'begin': /* 初始化进度条 total/actions */
    case 'task_start': /* 日志面板加行 */
    case 'task_skipped': /* 标记跳过 + reason */
    case 'task_done': /* 用 payload.outcome 着色（六态徽标） */
    case 'complete': /* 结束：payload.outcomes 做六态统计展示 */
  }
}, ac.signal)
```

### Step 3 — execute 接线
`index.tsx` L256 附近：

```ts
const summary = await useTauri().mole_optimize({
  dry_run: false,
  selected_actions, // Option<Vec<string>>，不传 = 全部执行
})
```

注意：命令耗时由 Rust 侧 `spawn_blocking` + 事件流式反馈，invoke 的 Promise 在全部完成后 resolve；期间 UI 状态全部由 Step 2 的事件驱动。

### Step 4 — 结果页渲染
- 逐任务结果：`results[].status` 即六态字符串，`error` 展示失败原因。
- 六态统计：`summary.outcomes`（`Partial<Record<OptimizeOutcomeStatus, number>>`）画徽标条。
- 空间统计：`stats.cache_cleaned_kb / databases_optimized / configs_repaired`。

### Step 5 — 清理
删除 `mock.ts` 及 index.tsx 中所有 `OPTIMIZE_MOCK_*` 引用。

> ⚠️ `useTauri` 的 Batcher 会对相同 key 的并发 invoke 去重合并。execute 期间避免以相同 payload 重复调用 `mole_optimize`（正常流程不会，仅提醒）。

---

## 6. 后端维护守则（改逻辑必读）

1. **Mole 是唯一权威**：任何任务逻辑/超时/结果点改动，先读 `/Users/liuy/mole_desktop/mvp/Mole/lib/optimize/` 对应实现再动手。
2. **三处一致性**：新增/删除任务必须同时改 `execute_optimization` 分派、`health_json::ROWS`、以及（Mole 侧同步时）`catalog.sh`。
3. **六态结果点比对法**（审计用）：
   ```bash
   # SH 侧
   awk 'index($0,"opt_xxx() {")==1 {in_fn=1} in_fn {if ($0~/optimize_task_result/) print; if ($0=="}") exit}' Mole/lib/optimize/tasks.sh
   # Rust 侧
   grep -n "OptimizeOutcome::" src-tauri/src/lib/optimize/tasks.rs
   ```
   两边的结果点（含条件）应一一对应；差异必须能解释（合并/拆分等价或平台差异）。
4. **fail-closed 原则**：探测类子命令异常时宁可任务 failed，不要静默继续或误删。
5. **验证命令**（`Molan2/src-tauri` 下）：
   ```bash
   cargo check          # 无 error、无 unused 警告
   cargo test --lib     # 当前 140 个测试全通过
   ```
6. **超时语义**：Rust 侧 `run_with_timeout` 超时返回 124；判定"超时/信号"统一 `rc == 124 || rc >= 128`（与 SH `[[ $rc -eq 124 || $rc -ge 128 ]]` 一致）。

---

## 7. 测试覆盖情况

- `outcome.rs`：`counts_resolve_like_shell`（折算规则）、`serde_names_match_outcomes_sh`（六态字符串）。
- `diagnostics.rs`：family 分类、挂载对解析（#960 回归）、detach 候选过滤等。
- `tasks.rs`：`trailing_slash_works` 等工具函数。
- 全量：`cargo test --lib` = 140 passed（含其他模块：clean/check/core 等）。
