# Molan 设置功能 — 扩展实施文档

> 基于腾讯柠檬 Cleaner 偏好设置分析，提炼出 Molan 应新增的设置项及实施步骤。
> 创建时间：2026-09-23 | 更新：2026-09-23（废纸篓清理提醒 ✅ 已完成）

---

## 一、柠檬偏好设置全貌

柠檬偏好设置窗口分两个 Tab：

### Tab 1：通用（PreferenceViewController）

| 设置组 | 内容 | 存储 |
|--------|------|------|
| 语言设置 | 中文 / English，切换后重启应用 | `LanguageHelper` |
| 主题设置 | 浅色 / 深色 / 跟随系统（macOS 10.14+） | `K_THEME_MODE_SETTED` |
| **自动检测卸载残留** | Toggle；监听 App 进入废纸篓 → 扫描残留 → 通知清理 | `IS_ENABLE_TRASH_WATCH` |
| **废纸篓清理提醒** | Toggle + 策略（删除时提醒 / 超过 500MB/1GB/2GB 时提醒） | `IS_ENABLE_TRASH_SIZE_WATCH` |
| 关闭主面板 | 最小化到 Dock / 直接退出 | `DOCK_ON_OFF_STATE` |

### Tab 2：状态栏（LMPreferenceStatusBarViewController）

| 设置项 | 内容 |
|--------|------|
| 启用状态栏 | 总开关 |
| 打开主界面时显示 | 子开关 |
| 开机时显示 | 子开关 |
| 展示信息 | Logo / 内存 / 磁盘 / CPU温度 / 风扇 / 网速 / CPU / GPU（多选） |

---

## 二、逐项分析 → Molan 决策

### ✅ 采纳：自动检测卸载残留

**柠檬机制**：状态栏进程监听 `NSWorkspace` 事件，检测到 `.app` 被拖入废纸篓后扫描 `~/Library` 残留。需要 Full Disk Access + 状态栏常驻。

**Molan 优势**：
- 不需要独立进程（Tauri 托盘常驻）
- 不需要 Full Disk Access（已能读 `~/Library`）
- 已有 `mole_orphan_scan` 后端命令

**适配方案**：
- Rust 侧用 `notify` crate（或 FSEvent）监听 `~/.Trash/` 目录
- 检测到新 `.app` → 调用 `orphan_scan` 扫描关联残留
- 通过 Tauri 事件 `EVT_RESIDUAL_DETECTED` 通知前端
- 前端弹确认条："检测到 XX 已卸载，是否清理残留文件？"

**设置项**：`uninstall.autoDetectResidual`，默认 `true`

---

### ✅ 采纳：废纸篓清理提醒

**柠檬机制**：后台线程定期 stat 废纸篓大小，超阈值提醒。两种策略：删除时 / 超阈值。

**Molan 适配**：
- 不单独起后台线程——复用 Dashboard 的 `status` 采集周期
- 后端 `status::snapshot` 加 `trash_size` 字段（扫描 `~/.Trash/` 目录大小）
- 前端 Dashboard 检测超阈值 → 底部弹通知条
- 简化为单一策略：**超阈值提醒**（柠檬的"每次删除时提醒"太频繁）

**设置项**：
- `trashReminder.enabled`：默认 `true`
- `trashReminder.threshold`：默认 `1024`（MB），可选 512 / 1024 / 2048

---

### ❌ 排除：语言设置

**理由**：当前 Molan 只有中文，没有多语言包。设置项加了也空转。等真正有英文包时再实现（React i18n 切换比柠檬简单，不需要重启）。

### ❌ 排除：主题设置

**理由**：Molan 固定渐变色体系，不支持自定义。

### ❌ 排除：关闭主面板行为

**理由**：Molan 已有多层退出策略（红叉隐藏 / Dock 隐藏 / Cmd+Q 真退 / 托盘真退），由 `app_menu.rs` + `macos_dock_quit.rs` 统一管控，不需要用户选择。柠檬需要这个设置是因为它没有退出拦截机制。

### ❌ 排除：状态栏展示信息

**理由**：Molan 的 Dashboard 气泡是固定展示（CPU/内存/磁盘/网络），不需要逐项配置。

---

## 三、新增设置项清单（汇总）

在 P0 已实现的基础上，新增以下设置项：

### 卸载分组（UninstallSection）

| 设置项 | key | 类型 | 默认值 | 优先级 |
|--------|-----|------|--------|--------|
| 自动检测卸载残留 | `uninstall.autoDetectResidual` | boolean | `true` | P1 |
| 显示系统应用 | `uninstall.showSystemApps` | boolean | `false` | P0（已定义） |
| 保留历史天数 | `uninstall.historyRetention` | number | `30` | P0（已定义） |

### 通用分组（GeneralSection）新增

| 设置项 | key | 类型 | 默认值 | 优先级 |
|--------|-----|------|--------|--------|
| 废纸篓清理提醒 | `trashReminder.enabled` | boolean | `true` | P2 |
| 提醒阈值 | `trashReminder.threshold` | number | `1024` | P2 |

---

## 四、分步实施计划

### 步骤 1：卸载设置 Section 骨架（前端）

**改动文件**：
- `src/pages/Settings/sections/UninstallSection.tsx`（新建）
- `src/pages/Settings/index.tsx`（替换占位符）

**内容**：
- "自动检测卸载残留" Switch + 描述文案
- "显示系统应用" Switch
- "保留历史天数" Select（7天 / 30天 / 90天 / 永久）

**依赖**：useSettings hook 中已有 `uninstall.showSystemApps` 和 `uninstall.historyRetention` 的定义，需要加 `uninstall.autoDetectResidual`。

---

### 步骤 2：自动检测残留 — 后端 Rust 实现

**新建模块**：`src-tauri/src/lib/uninstall/residual_watch.rs`

**核心逻辑**：
```rust
// 1. 用 notify crate 监听 ~/.Trash/ 目录
// 2. 检测新增 .app 目录
// 3. 提取 bundleId（从 Info.plist）
// 4. 调用 orphan_scan 扫描 ~/Library 关联残留
// 5. emit EVT_RESIDUAL_DETECTED 事件（含 appName + residual_paths）
```

**新增 Rust 命令**：
- `mole_residual_watch_start()` — 启动监听（设置开启时调用）
- `mole_residual_watch_stop()` — 停止监听

**新增事件**：
- `EVT_RESIDUAL_DETECTED` — payload: `{ appName, paths, totalSize }`

**Cargo.toml**：新增 `notify = "6"` 依赖

---

### 步骤 3：自动检测残留 — 前端对接

**改动文件**：
- `src/pages/Uninstall/index.tsx`（监听 EVT_RESIDUAL_DETECTED 弹确认条）
- `src/constants/tauri-events.ts`（注册事件名）
- `src/constants/tauri-commands.ts`（注册命令名）

**交互流程**：
1. 用户拖 App 到废纸篓
2. 后端检测到 → emit `EVT_RESIDUAL_DETECTED`
3. 前端 Uninstall 页收到事件 → 顶部弹通知条
4. 用户点击"扫描残留" → 调用 `mole_orphan_scan`
5. 展示结果 → 用户确认 → 调用 `mole_orphan_delete`

---

### 步骤 4：废纸篓大小采集（后端）

**改动文件**：`src-tauri/src/lib/core/common.rs`（或 status 模块）

**核心逻辑**：
```rust
// 扫描 ~/.Trash/ 目录大小（jwalk 并行遍历）
pub fn trash_size() -> u64 {
    let trash = dirs::home_dir().unwrap().join(".Trash");
    jwalk::WalkDir::new(&trash)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}
```

**集成点**：在 `status::snapshot` 返回结构中加 `trash_size: u64` 字段。

---

### 步骤 5：废纸篓清理提醒（前端）

**改动文件**：
- `src/pages/Dashboard/index.tsx`（检测超阈值 → 弹通知条）
- `src/pages/Settings/sections/GeneralSection.tsx`（加 Switch + Select）
- `src/pages/Settings/useSettings.ts`（加 `trashReminder` 字段）

**交互流程**：
1. Dashboard 每帧检查 `snap.trash_size` 与阈值
2. 超过阈值 → 底部弹通知条："废纸篓已占用 X GB，[前往清理]"
3. 点击"前往清理" → 调用系统 Finder 清废纸篓 或 导航到 Clean 页

---

### 步骤 6：清理/高级 Section 实现（P2+）

**CleanSection**：
- 默认删除方式（废纸篓 / 直接删除）
- 默认扫描口径（逻辑 / 物理）
- 白名单管理快捷入口

**AdvancedSection**：
- 清除扫描缓存
- 恢复所有默认设置
- 打开配置目录

---

## 五、数据流架构

```
设置页面                    tauri-plugin-store              Rust 后端
   │                          (settings.json)                  │
   ├─ updateSetting() ────→ store.set(key, val) ────→ settings.json
   │                          ↑                                  │
   ├─ toggleAutoLaunch() ─────────────────────→ mole_auto_launch_toggle()
   │                                                          (LoginItems)
   │                                                            │
   ├─ 自动检测残留 ──────────────────────────→ residual_watch_start()
   │     ←──── EVT_RESIDUAL_DETECTED ←─────── notify::Watcher
   │                                                            │
   └─ 废纸篓提醒 ←──── snap.trash_size ←──── status::snapshot()
                                               ↓
                                         trash_size 采集
```

---

## 六、settings.json 完整结构（当前态）

```json
{
  "autoLaunch": true,
  "autoCheckUpdate": true,
  "updateCheckInterval": 1,
  "clean": {
    "deleteMode": "trash",
    "sizeMetric": "logical"
  },
  "uninstall": {
    "historyRetention": 30,
    "showSystemApps": false
  },
  "trashReminder": {
    "enabled": true,
    "threshold": 1024
  },
  "appVersion": {
    "lastCheckedAt": "...",
    "dismissedVersion": null,
    "consecutiveFailures": 0
  }
}
```

---

## 七、实施优先级总览

| 阶段 | 内容 | 估计工作量 |
|------|------|-----------|
| **P0**（已完成） | 窗口框架 + 通用设置 + 入口接入 | ✅ 已完成 |
| **P1** | 卸载设置 Section + 自动检测残留（Rust 监听 + 前端通知） | ~2-3 天 |
| **P2** | 废纸篓提醒 + 清理设置 + 高级选项 | ~1-2 天 |
| **P3**（远期） | Cmd+, 快捷键 + 设置变更实时生效 + 多语言（如有需求） | ~1 天 |
