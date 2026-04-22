用户提了一个结构化的日志埋点方案，我先做一轮代码调研确认几个关键实现点（磁盘空间获取方式、现有日志写入机制、clean_apply 的入参结构），再给出可行性判断并实施。
关键信息已确认：项目已有 `boot_volume_free_bytes()`（`df -k` 封装）和 `get_free_space()`（`df -h` 封装），可直接复用，无需新增 shell 调用。接下来确认快照结构和 clean_apply 的入参细节。
方案调研完成，整体可行，我已经把三个阶段的插入点和数据来源都定位好了。有三处微调需要说明（不改变你的目标）：

1. **df 快照**：不新增 shell 调用——项目里已有 `boot_volume_free_bytes()`（内部即 `df -k`）和 `get_free_space()`（`df -h`），直接复用，效果与你要求的一致，也符合项目"禁止新增外部二进制调用"的红线。
2. **"前端勾选结果"的采集时点**：后端在扫描阶段拿不到用户最终勾选（前后端跨进程，勾选状态只在前端内存）。修正方案：scan 日志记 `default_selected`（默认勾选）汇总作对照；**清理日志**从快照 `items` + `clean_apply` 入参 `item_ids` 推出**全量 item 的 `selected=true/false`** 并记勾选总量——这才是"用户最终点清理时"的权威勾选数据。
3. **触发点**已定位：scan → `mole_clean` 收尾（快照构建处，刚好能拿到 `scan_id`、全部 categories、`initial_free_bytes`）；clean/verify → `clean_apply`（逐分类实时追加 + 收尾写验证）。

其余按你的要求做：JSONL 格式、每条含时间戳/会话 ID/操作类型、5MB 轮转保留 3 个备份、默认开启、写入失败静默不影响主流程。

动手前有一个需要你定的点——你列了 `scan.log / clean.log / verify.log` 三个名字，但路径又写的是 `debug-{timestamp}.log`，两者需要融合一下：
方案确定：**单文件批次**。开始实施。先看几个关键位置的现状（模块注册方式、`mole_clean` 与 `clean_apply` 的上下文、现有导入）。
开始实施。先确认 `base.rs` 中几个工具函数的签名（`ensure_user_file` / `get_file_size` / `home_dir` / `bytes_to_human`），确保新模块能正确复用。
开始实施。先看一下 `clean_apply` 尾部与另一个相似函数（`mole_clean_paths`）的区别，确保编辑定位唯一。
定位确认完毕（`clean_apply` 尾部有 `emit_clean_apply_progress` 唯一标识）。现在创建调试埋点模块 `debug_trace.rs`。
模块创建完成。现在注册模块并添加导入。
现在处理 `clean_apply` 的四处埋点。先确认 `bytes_to_human` 在 clean.rs 中的导入方式。
`bytes_to_human` 是 clean.rs 的本地函数，可直接使用。继续插入 `clean_apply` 的四处埋点。
所有埋点插入完成。现在验证插入位置正确、并确认 `execute_category_items` 的返回类型（`error` 是否为 `Option<String>`）。
`error` 类型确认无误（`Option<String>`，`as_deref()` 正确）。但注意到排版可能有缩进异常，读取实际文件内容确认。
scan 部分缩进正确（grep 输出只是显示截断）。继续确认 `clean_apply` 的几处。
所有埋点插入位置和缩进正确。现在编译验证。
编译通过，0 error，7 个既有 warning（无新增）。现在运行新模块的单元测试。
测试通过。再补一个真实写盘路径的测试（`append_jsonl` 的落盘与 JSONL 行格式），并确认前端扫描入口确实经过我埋点的代码路径。
路径确认：`clean_scan` → `mole_clean(dry_run=true)`，埋点在真实链路上。补充写盘测试。
修复上一处编辑造成的换行问题，并正确插入写盘测试。
4/4 通过。跑全量测试套件 + 静态检查，确认没有破坏其它部分。
193/194 通过。有 1 个失败位于 `cmd/analyze/delete.rs`（不在本次改动范围内）——查看失败详情，确认是否为既有失败。
失败测试确认为环境性失败（`trash binary not found`，与本次改动无关的既有测试依赖）。查看最终改动规模并记录本次机制。
改动规模确认：`clean.rs` 相对 HEAD 共 +166/-18（包含 P1 遗留改动）。快速检查 diff 中的删除行是否均为预期。
全部完成并验证通过。以下是本次实现的总结。

---

## 已完成：三阶段调试埋点日志

### 改动清单（前端零改动）

| 文件 | 改动 |
|---|---|
| `src-tauri/src/lib/core/debug_trace.rs` | **新增**（466 行）：JSONL 埋点模块 + 4 个单测 |
| `src-tauri/src/lib/core/mod.rs` | 注册 `pub mod debug_trace;` |
| `src-tauri/src/controllers/clean.rs` | 插入埋点（纯新增行，无删除） |

### 各阶段写入内容与触发时机

| 阶段 | 触发点 | 记录内容 |
|---|---|---|
| **scan** | `mole_clean(dry_run)` 快照存储后 | `disk_before`（扫描前可用空间）→ 逐 item（`category_id`/`item_id`/`path`/`real_path`/`size_kb`/`file_count`/`status`/`default_selected`）→ `default_selection_summary`（默认勾选量与总量）→ `summary`（可清理总量/分类数/文件数/耗时） |
| **clean** | `clean_apply` 执行期间 | `disk_before`（清理前空间）→ **全量 item 的 `selected=true/false`**（从快照+入参推出，这是"用户最终勾选"的权威清单）→ `selection_summary`（勾选总量 + 后端二次校验拦下量 + 快照全量对照）→ 逐分类 `category_result`（实时，含 `size_cleaned`/`status`/`error`）→ `summary` |
| **verify** | `clean_apply` 完成后 | `disk_after`（清理后空间）→ `verdict`：`difference_bytes = free_space_change - total_cleaned_size`（**负值 = 报告释放多于磁盘实际变化**，即你之前遇到的"对不上"特征） |

### 文件机制（按你确认的单文件批次方案）

- 批次文件：`~/Library/Logs/mole/debug-{timestamp}.log`，每次 dry-run 扫描时创建；clean/verify 追加到同一批次（跨进程时自建兜底文件）
- 单文件超 5MB 自动轮转，保留 `.1/.2/.3` 三个备份
- 每行 JSON 含公共字段 `ts` / `op`(scan|clean|verify) / `session_id` / `event`；写入失败静默，默认开启

行格式示例（示意）：

```json
{"op":"scan","session_id":"scan-19a2...","event":"default_selection_summary","selected_items":66,"selected_size_bytes":801000000,"selected_size_human":"800.1MB",...}
{"op":"clean","session_id":"clean-19a2...","event":"selection_item","item_key":"dev_tools::dev_npm","size_bytes":39800000,"selected":true,"status":"cleanable"}
{"op":"clean","session_id":"clean-...","event":"category_result","category_id":"dev_tools","item_ids":["dev_npm"],"size_cleaned":123456,"status":"cleaned","error":null}
{"op":"verify","session_id":"clean-...","event":"verdict","total_cleaned_size":515000000,"free_space_change":2048000,"difference_bytes":-512952000,"difference_human":"-512.9MB"}
```

### 与原始方案的 3 处调整（事前已说明）

1. **df 快照**：复用 `boot_volume_free_bytes()`（内部即 `df -k`）同源值，未新增 shell 调用
2. **"前端勾选结果"**：权威数据在 `clean_apply` 入参——已实现为 clean 阶段的全量 `selected=true/false` 清单；scan 阶段记 `default_selected` 作对照（可看出用户手动调整了多少）
3. **执行结果的 `path`**：`execute_category_items` 接口不返回路径，以 `item_ids` 数组代替（每项路径可在 scan 日志里按 `item_key` 反查）

### 验证结果

- `cargo check`：**0 error**（7 个既有 warning 不变，无新增）
- `cargo test --lib core::debug_trace`：**4/4 通过**（session id、JSONL 行合法性、真实写盘、轮转备份）
- 全量 `cargo test --lib`：193/194；唯一失败 `test_move_to_trash_via_binary` 为环境性失败（`trash binary not found`，与本次改动无关）

### 下次运行后这样读

重新构建运行应用（前端无需改动），执行一次"扫描 → 清理"，然后：

```bash
# 最新批次文件
ls -lt ~/Library/Logs/mole/debug-*.log | head -3

# 一屏看关键汇总（三组数据对比入口）
grep -E '"event":"(disk_before|default_selection_summary|selection_summary|summary|disk_after|verdict)"' ~/Library/Logs/mole/debug-*.log

# 逐分类执行明细（找出被 skip/failed 的量）
grep '"event":"category_result"' <批次文件>
```

对比逻辑：`selection_summary.selected_size_bytes`（你勾选的）vs 各 `category_result` 汇总（实清）vs `verdict.difference_bytes`（磁盘是否真释放）。这样之前"800 → 491 → 磁盘不动"每一环差在哪就都有据可查了。