
好，我来逐项分析这份 todo.md 中各借鉴点的价值。文件包含三个参考项目的借鉴项，我按**推荐优先级**分三档来说。

---

## 第一档：强烈推荐做（高价值 + 低成本）

### 1. Dock 退出拦截（dbx，第 776-957 行）

**值得做的理由：**
- 这是**当前唯一真实退出路径的安全漏洞**。主窗口关闭走"隐藏到托盘"，托盘没有退出菜单，Dock 退出直接杀进程——全盘扫描跑到 15 分钟时用户 Dock 退出就全废了
- 实现成本极低：~70 行 Rust + ~40 行前端，总共约 110 行
- 对清理软件来说，中断保护是**基本体验底线**，不是锦上添花
- 方案成熟，dbx 已验证过，且代码骨架已经写好了

**唯一风险**：Tauri 2.x 的 tao 版本 delegate 类名可能不叫 `TaoAppDelegateParent`，需上机验证。但即使名字不同，改一行就够。

### 2. keep-awake 防止系统休眠（Gyroflow 第五节，第 280-334 行）

**值得做的理由：**
- 全盘扫描 / 深度清理 / 批量卸载都可能跑几十分钟，macOS 默认空闲几分钟就休眠，**任务会直接停滞**
- 实现极薄：crates.io 的 `keep-awake` crate，RAII guard，约 30 行包装代码
- 接入点清晰（3 个 controller 入口各一行），无侵入性
- MAS 沙盒内 IOKit assertion 可用，不影响合规

### 3. PlatformInfo 安装形态检测（花笺第二节，第 615-757 行）

**值得做的理由：**
- 这是**架构布局**，不是功能点。把 `DistributionChannel` 做成一等公民，未来所有渠道差异（sudo 降级、trash 行为、自更新开关）只查一个统一出口
- 现在不提取，将来每个功能模块里散落 `cfg!(feature = "mas")` 判断，维护成本指数级增长
- 代码量小（~80 行），且是后续自更新、沙盒适配等地基
- 花笺 MIT 协议，代码可直接参考

---

## 第二档：值得做但需把握时机

### 4. Store Package 检测（Gyroflow 第三节，第 164-234 行）

**分析**：思路本身很好（运行时检测 `APP_SANDBOX_CONTAINER_ID` 环境变量，10 行代码），但已被 PlatformInfo（第二节）完全吸收取代。文档里自己也写了"第三节骨架已被第二节取代"。所以**不需要单独做**，做 PlatformInfo 时一并实现即可。

### 5. 自实现更新器系统（花笺第一节，第 375-611 行）

**值得做但时机未到，文档自己的判断是对的：**
- MAS 版不能自更新，官网版的更新源还没定，现在做是空转
- 但分析质量极高：Zed（极简无回滚）vs 花笺（完整回滚 4000 行）的对比、混合方案（Zed 骨架 + ditto + 最简回滚 ~300-400 行）已经设计好了
- **建议**：上线准备阶段直接按 1.5 的方案实施，不用再分析。其中 `keyring` 存 license、`json_io` 原子写入这两个小件可以本版本顺手做

### 6. fs_gate 文件系统门卫层（Gyroflow 第二节，第 110-161 行）

**有价值但优先级低于上面几项：**
- 把"路径能不能摸"的判断收敛到一个模块，为 MAS 沙盒留插槽——思路正确
- 官网版下是 no-op，不会引入回归风险
- 但当前 MAS 上架还远，沙盒适配可以再等等。**建议在 Dock 退出拦截 + keep-awake + PlatformInfo 做完后再铺这层**，1 天工作量

---

## 第三档：当前不值得做

### 7. macOS 沙盒安全作用域资源管理（Gyroflow 第一节，第 19-107 行）

**暂不值得做：**
- 文档自己标了"待定"，三个前置问题（sudo 沙盒冲突、哪些功能需授权、trash 沙盒行为）都没验证
- 实现复杂度高：引用计数、延迟关闭线程、书签持久化、entitlements 配置
- 依赖 MAS 上架决策，而 MAS 上架被推迟了
- **建议**：先做 fs_gate 的薄门卫层留好插槽，沙盒实现等 MAS 决策明确后再动

### 8. parking_lot 替换 std::sync（Gyroflow 第四节，第 238-277 行）

**不值得做：**
- 全库**只有一处锁**（`controllers/clean.rs:87`），替换收益接近零
- 文档自己的决策规则写得很清楚：锁数量 ≥ 3 处或出现锁热点才引入
- 当前引入纯属增加依赖，没有实际收益

---

## 总结排序

| 优先级 | 项目 | 工作量 | 理由 |
|--------|------|--------|------|
| **P0** | Dock 退出拦截 | ~1 天 | 安全漏洞，成本极低，收益极高 |
| **P0** | keep-awake | ~半天 | 长任务休眠是真实痛点，实现极薄 |
| **P1** | PlatformInfo | ~半天 | 架构地基，越早提取越好 |
| **P2** | 自更新的小件（keyring / json_io） | ~半天 | 上线前顺手做，与主流程解耦 |
| **P2** | fs_gate 门卫层 | ~1 天 | 为 MAS 留插槽，但不急 |
| **P3** | 自更新主系统 | ~1 周 | 上线阶段再实施 |
| **P3** | 沙盒安全作用域 | 视考察结论 | 等 MAS 决策明确 |
| **不做** | Store Package 检测 | — | 已被 PlatformInfo 吸收 |
| **不做** | parking_lot | — | 一处锁，无收益 |

整体来看，这份 todo 的分析质量很高，尤其是自更新系统的对比分析和 Dock 退出拦截的方案设计都已经可以直接落地。**最值得立刻做的是 P0 的两项**：Dock 退出拦截和 keep-awake，都是低成本高回报、且解决真实痛点的。


让我先看看当前已实现的自更新代码，确认实际状态。


现在我对全局情况清楚了。让我做一个完整分析。

---

## 分析结论

### 自更新主系统：已经实现，且比 todo.md 方案更优

你的实际实现选择了 **`tauri-plugin-updater`**（Tauri 2 官方插件），而非 todo.md 中设计的自定义方案（hdiutil + ditto + rename 回滚）。对比一下：

| 维度 | todo.md 方案（花笺+Zed 混合） | 实际实现（tauri-plugin-updater） |
|------|------|------|
| 代码量 | 预估 300-400 行核心 | 后端 ~450 行（含 Brew 兼容逻辑），但核心更新逻辑仅 ~190 行 |
| 签名校验 | 需自己实现 SHA-256 | 内置 Ed25519，开箱即用 |
| 回滚机制 | rename .backup + 还原 | 插件内部处理（无显式回滚，但 tar.gz 原子解压更安全） |
| 更新源 | 需自建 HTTP client + fallback | 配置 endpoints 数组，插件自动 fallback |
| 依赖数 | reqwest + sha2 + semver + tempfile | 仅 `tauri-plugin-updater = "2"` 一行 |
| 维护成本 | 高（自定义安装流程、DMG 卸载、回滚逻辑） | 低（官方维护，社区活跃） |

**结论：实际方案完胜 todo.md 方案。** 官方插件处理了签名校验、下载、安装、进度回调、双源 fallback，代码量更少、维护成本更低。todo.md 里设计的 ditto + rename 回滚方案虽然精巧，但对于 Tauri 应用来说属于过度工程——官方插件已经把这些问题解决了。

### 小件（keyring / json_io）的适用性重新评估

todo.md 中提到的两个"顺手做"小件：

#### 1. `keyring` 存 license/激活信息

**当前不需要，未来按需。**

- 目前 MoleStudio 没有 license/付费体系，`keyring` 没有消费场景
- 等将来确定付费模式（买断 / 订阅 / 免费）后再决定是否需要
- 如果做，花笺的 `cdk_store.rs`（84 行）仍是好的参考

#### 2. `json_io` 原子写入

**已被 `tauri-plugin-store` 替代。**

- 花笺的 `json_io`（tmp + sync_all + rename + sync_parent_dir）解决的是配置文件写入中途崩溃导致数据损坏
- 你的项目已经用 `tauri-plugin-store`（`settings.json`）做持久化，插件内部已处理原子写入
- 自更新的检查结果、dismissed 版本都存在 store 里，不需要额外的原子写入层

### 当前自更新系统的完成度

从代码看，核心链路已完整：

| 环节 | 状态 | 文件 |
|------|------|------|
| 后端检查命令 | 已完成 | [app_version.rs](file:///home/a16/work/molestudio/src-tauri/src/lib/manage/app_version.rs) |
| 后端安装命令 | 已完成 | [app_version.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/app_version.rs) |
| MAS 检测 | 已完成 | `is_mas_build()` + App Store 引导 |
| 前端 hook | 已完成 | [useAppVersion.ts](file:///home/a16/work/molestudio/src/hooks/useAppVersion.ts) |
| UI 交互（红点+菜单） | 已完成 | [BottomBar.tsx](file:///home/a16/work/molestudio/src/pages/Dashboard/BottomBar.tsx) |
| 前端契约（命令/事件/类型） | 已完成 | tauri-commands.ts / tauri-events.ts / mole.ts |
| Gitee + GitHub 双源 | 已配置 | tauri.conf.json → plugins.updater.endpoints |

**上线前待办**（文档 6.1-6.4 节列得很清楚）：
1. 生成 Ed25519 签名密钥，替换 pubkey 占位符
2. 在 Gitee 仓库创建 `update/latest.json`
3. 走通一次完整发版流程（build → sign → release → latest.json）
4. 验证清单（真机检查/安装/MAS 构建/断网降级）

### 总结

| 项目 | 结论 |
|------|------|
| 自更新主系统 | **已完成**，且方案优于 todo.md 设计，无需再改 |
| keyring | **暂不需要**，等付费模式确定 |
| json_io 原子写入 | **已被 tauri-plugin-store 替代**，不需要 |
| todo.md 自更新章节 | 可以标记为"已完成，方案升级为 tauri-plugin-updater"，留档参考 |

你现在的自更新系统已经很完整了。剩下的工作全部是**上线运维侧**（签名密钥、latest.json、发版流程），不是代码层面的。


让我先摸清当前代码现状，再给出具体实施方案。


Now let me check the specific long-running task entry points and the existing `SCAN_REGISTRY` lock:


Now let me check the exact entry points for the four long-running tasks:


现在我对整个代码库有了完整的了解。以下是三个项目的详细实施方案分析。

---

## 一、P0 — Dock 退出拦截

### 1.1 问题现状

当前代码的退出路径分析：

| 路径 | 行为 | 有无拦截 |
|------|------|----------|
| 主窗口红绿灯关闭 | [lib.rs L173-176](file:///home/a16/work/molestudio/src-tauri/src/lib.rs#L173-L176) `prevent_close → hide` | 有，隐藏到托盘 |
| BottomBar 退出按钮 | [BottomBar.tsx L33-39](file:///home/a16/work/molestudio/src/pages/Dashboard/BottomBar.tsx#L33-L39) `destroy()` 全部窗口 | 无拦截，直接杀进程 |
| **Dock 右键 → 退出** | AppKit `applicationShouldTerminate:` | **完全无拦截**，直接杀进程 |

后两条路径在扫描跑到 15 分钟时，用户一碰就全废了。

### 1.2 实施方案（按 todo.md dbx 方案适配）

**新增文件 1：`src-tauri/src/lib/core/busy_state.rs`（~25 行）**

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

static BUSY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 长任务入口调用，返回 RAII guard，任务结束自动 -1
pub fn enter_busy() -> BusyGuard {
    BUSY_COUNT.fetch_add(1, Ordering::SeqCst);
    BusyGuard
}

pub fn is_busy() -> bool {
    BUSY_COUNT.load(Ordering::SeqCst) > 0
}

pub struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) { BUSY_COUNT.fetch_sub(1, Ordering::SeqCst); }
}
```

**挂接点**（4 个 `spawn_blocking` 入口，每个加一行 `let _busy = busy_state::enter_busy();`）：

| 文件 | 行 | 函数 |
|------|------|------|
| [controllers/analyze.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/analyze.rs#L302) | L302 | `mole_analyze` → `spawn_blocking` 闭包内 |
| [controllers/clean.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/clean.rs#L1643) | L1643 | `mole_clean_execute` → `spawn_blocking` 闭包内 |
| [controllers/uninstall.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/uninstall.rs#L1250) | L1250 | `mole_uninstall_batch` → `spawn_blocking` 闭包内 |
| [controllers/optimize.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/optimize.rs#L272) | L272 | `mole_optimize` → `spawn_blocking` 闭包内 |

**新增文件 2：`src-tauri/src/macos_dock_quit.rs`（~70 行）**

核心逻辑：
1. `install_dock_quit_handler()` — 在 setup 中调用，用 `class_addMethod` 给 `TaoAppDelegateParent` 注入 `applicationShouldTerminate:`
2. 回调逻辑：`CONFIRMED_EXIT` 为 true → 放行；`is_busy()` 为 false → 放行（安静退出）；有任务 → emit `dock-quit-requested` 事件 + 取消终止
3. `confirm_dock_quit` command — 前端确认后调用，置标志 + `app.exit(0)`

**需要改的现有文件：**

| 文件 | 改动 |
|------|------|
| [Cargo.toml](file:///home/a16/work/molestudio/src-tauri/Cargo.toml#L82) | `objc2-app-kit` features 加 `"NSApplication"` |
| [lib.rs](file:///home/a16/work/molestudio/src-tauri/src/lib.rs#L219) | setup 内加 `install_dock_quit_handler(handle)`；`lib.rs` 顶部加 `mod macos_dock_quit` |
| [lib/core/mod.rs](file:///home/a16/work/molestudio/src-tauri/src/lib/core/mod.rs) | 加 `pub mod busy_state;` |
| [events.rs](file:///home/a16/work/molestudio/src-tauri/src/events.rs) | 加 `dock-quit-requested` 事件常量 |
| [tauri-events.ts](file:///home/a16/work/molestudio/src/constants/tauri-events.ts) | 加 `EVT_DOCK_QUIT_REQUESTED` |
| [tauri-commands.ts](file:///home/a16/work/molestudio/src/constants/tauri-commands.ts) | 加 `CMD_CONFIRM_DOCK_QUIT` |
| [layout/index.tsx](file:///home/a16/work/molestudio/src/layout/index.tsx) | 监听 `dock-quit-requested` → 弹确认框 → 调 `confirm_dock_quit` |

### 1.3 关键风险

**最大风险**：`TaoAppDelegateParent` 类名是否匹配我们的 Tauri 2 版本。需要在 macOS 上机验证。如果类名不同，从 tao 源码 `tao/src/platform_impl/macos/app_delegate.rs` 查实际类名即可，改一行。

**备选方案**：如果类名注入方案不可行，Tauri 2 本身有 `on_exit_requested` 事件（`RunEvent::ExitRequested`），可以在 [lib.rs L219](file:///home/a16/work/molestudio/src-tauri/src/lib.rs#L219) 的 `.run()` 回调里拦截。但此方案不如注入方案精细（无法区分"空闲退出"和"忙碌退出"），作为降级方案。

---

## 二、P0 — keep-awake 防止系统休眠

### 2.1 问题现状

当前代码中 4 个长任务（analyze/clean/uninstall/optimize）都是 `spawn_blocking` 执行，没有任何防休眠机制。macOS 默认空闲几分钟后 AppNap/休眠，扫描进度直接停滞。

### 2.2 实施方案

**方案选择**：

| 方案 | 优点 | 缺点 |
|------|------|------|
| A. `keep-awake` crate | 跨平台、API 干净 | 可能拉入旧版 objc2 与现有 0.6.4 冲突 |
| B. 手写 objc2 + IOKit | 零新依赖、完全可控 | 约 30 行 FFI 代码 |

**推荐方案 B**（手写），原因：
- 项目已有 `objc2 0.6.4` + `objc2-foundation 0.3.2`，不需要额外依赖
- IOKit 的 `IOPMAssertionCreateWithName` / `IOPMAssertionRelease` 是纯 C API，用 `extern "C"` 声明即可，不需要 objc2 参与
- 代码量极小（~40 行），完全可控，无供应链风险

**新增文件：`src-tauri/src/lib/core/keep_awake.rs`（~40 行）**

```rust
// macOS IOKit 防休眠。纯 C FFI，零新依赖。
#[cfg(target_os = "macos")]
mod inner {
    use std::ffi::CString;

    // IOKit C API 声明
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const std::ffi::c_void,
            timeout: u32,
            assertion_name: *const std::ffi::c_char,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    const K_IOP_ASSERT_PREVENT_SYSTEM_SLEEP: &str = "PreventUserIdleSystemSleep";

    pub struct KeepAwakeGuard { assertion_id: u32 }

    impl KeepAwakeGuard {
        pub fn acquire(reason: &str) -> Option<Self> {
            let type_cstr = CString::new(K_IOP_ASSERT_PREVENT_SYSTEM_SLEEP).ok()?;
            let name_cstr = CString::new(reason).ok()?;
            let mut id: u32 = 0;
            let status = unsafe {
                IOPMAssertionCreateWithName(
                    type_cstr.as_ptr() as *const _,
                    0,
                    name_cstr.as_ptr(),
                    &mut id,
                )
            };
            if status == 0 { Some(Self { assertion_id: id }) }
            else { None }
        }
    }

    impl Drop for KeepAwakeGuard {
        fn drop(&mut self) {
            unsafe { IOPMAssertionRelease(self.assertion_id); }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod inner {
    pub struct KeepAwakeGuard;
    impl KeepAwakeGuard {
        pub fn acquire(_reason: &str) -> Option<Self> { Some(Self) }
    }
}

pub use inner::KeepAwakeGuard;
```

**挂接方式**：与 busy_state 相同的 4 个 `spawn_blocking` 入口，在闭包开头加：

```rust
let _awake = KeepAwakeGuard::acquire("Disk analysis"); // reason 按任务不同
```

| 挂接点 | reason |
|------|------|
| `mole_analyze` | `"Disk analysis"` |
| `mole_clean_execute` | `"Deep cleaning"` |
| `mole_uninstall_batch` | `"Uninstalling apps"` |
| `mole_optimize` | `"System optimization"` |

**需要改的现有文件：**

| 文件 | 改动 |
|------|------|
| [lib/core/mod.rs](file:///home/a16/work/molestudio/src-tauri/src/lib/core/mod.rs) | 加 `pub mod keep_awake;` |
| 4 个 controller 文件 | spawn_blocking 闭包内加 `let _awake = ...` |

### 2.3 验证方式

```bash
# macOS 上发起全盘扫描，另一终端：
pmset -g assertions
# 预期出现：PreventUserIdleSystemSleep，Details 里 app 名 MoleStudio

# 扫描结束后再查：断言消失（RAII drop 生效）
```

---

## 三、P1 — PlatformInfo 安装形态检测

### 3.1 为什么是架构地基

当前 `is_mas_build()` 在 [app_version.rs L59](file:///home/a16/work/molestudio/src-tauri/src/lib/manage/app_version.rs#L59) 已经实现了 MAS 检测（通过 `_MASReceipt/receipt`），但它是**局部函数**，只服务于自更新模块。

PlatformInfo 要做的升级是：**把分散的渠道判断收敛成统一出口**，未来所有"官网版 vs MAS 版"的分支逻辑（sudo 降级、trash 行为、自更新开关、entitlements 差异）都只查 `PlatformInfo::current().channel`，不再到处散落 `cfg!(feature = "mas")` 或 `is_mas_build()`。

### 3.2 实施方案

**新增文件：`src-tauri/src/lib/platform/platform_info.rs`（~80 行）**

```rust
use serde::Serialize;
use std::{env, path::PathBuf, sync::OnceLock};

/// 分发渠道。新渠道（如 Homebrew cask）只扩这里 + detect_channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DistributionChannel {
    Direct,        // 官网直发
    MacAppStore,   // MAS（沙盒）
}

/// 运行形态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    MacosAppBundle, // 正式 .app 包
    Unknown,        // target/debug 或找不到 .app
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub channel: DistributionChannel,
    pub install_kind: InstallKind,
    pub app_version: String,
    pub current_app_bundle: Option<String>,
    pub sandboxed: bool,
    pub full_build: bool,
}

static CACHE: OnceLock<PlatformInfo> = OnceLock::new();

/// 全库统一出口：渠道分支只准查这里
pub fn current() -> &'static PlatformInfo {
    CACHE.get_or_init(compute)
}

fn compute() -> PlatformInfo { /* ... */ }

#[cfg(target_os = "macos")]
fn is_sandboxed() -> bool {
    env::var("APP_SANDBOX_CONTAINER_ID").is_ok()
}
#[cfg(not(target_os = "macos"))]
fn is_sandboxed() -> bool { false }

fn detect_channel(sandboxed: bool) -> DistributionChannel {
    #[cfg(feature = "mas")]
    { let _ = sandboxed; return DistributionChannel::MacAppStore; }
    #[cfg(not(feature = "mas"))]
    { if sandboxed { DistributionChannel::MacAppStore }
      else { DistributionChannel::Direct } }
}

fn find_macos_app_bundle(exe: &std::path::Path) -> Option<PathBuf> {
    let mut cur = exe.parent();
    while let Some(p) = cur {
        if p.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}
```

**与现有 `is_mas_build()` 的关系**：

`app_version.rs` 里的 `is_mas_build()` 可以保留不动（它是自更新模块内部函数），但 `PlatformInfo` 的 `channel` 字段提供的是**更高层的语义**——不只看 receipt 文件，还综合编译期 feature + 运行时沙盒检测。未来如果其他模块需要判断渠道，用 `PlatformInfo::current().channel` 而非直接调 `is_mas_build()`。

**需要改的现有文件：**

| 文件 | 改动 |
|------|------|
| [lib/platform/mod.rs](file:///home/a16/work/molestudio/src-tauri/src/lib/platform/mod.rs) | 加 `pub mod platform_info;` |
| [controllers/platform.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/platform.rs) | 加 `mole_get_platform_info` command |
| [lib.rs](file:///home/a16/work/molestudio/src-tauri/src/lib.rs#L69) | `invoke_handler` 加新命令 |
| [tauri-commands.ts](file:///home/a16/work/molestudio/src/constants/tauri-commands.ts) | 加 `CMD_MOLE_GET_PLATFORM_INFO` |
| [types/mole.ts](file:///home/a16/work/molestudio/src/types/mole.ts) | 加 `PlatformInfo / DistributionChannel / InstallKind` 类型 |

### 3.3 消费规范（写进代码注释）

任何功能要做渠道差异分支，**只准**：
```rust
// Rust 侧
if platform_info::current().channel == DistributionChannel::Direct { ... }
```
```typescript
// 前端侧
if (platformInfo.channel === 'direct') { ... }
```

**禁止**再散落 `cfg!(feature = "mas")` 或 `is_mas_build()` 的裸判断。

---

## 四、实施顺序与依赖关系

```
           ┌─────────────────────────┐
           │ ① busy_state.rs (25行)  │
           │ ② keep_awake.rs (40行)  │ ← 无依赖，可并行
           └──────────┬──────────────┘
                      │
           ┌──────────▼──────────────┐
           │ ③ macos_dock_quit.rs    │ ← 依赖 busy_state
           │   (70行 Rust + 前端)     │
           └──────────┬──────────────┘
                      │
           ┌──────────▼──────────────┐
           │ ④ platform_info.rs      │ ← 独立，随时可做
           │   (80行 Rust + 前端)     │
           └─────────────────────────┘
```

建议顺序：

| 步骤 | 内容 | 工作量 | 说明 |
|------|------|--------|------|
| 1 | `busy_state.rs` + 4 个 controller 挂接 | 30 分钟 | 纯增量，零风险 |
| 2 | `keep_awake.rs` + 4 个 controller 挂接 | 30 分钟 | 纯增量，零风险 |
| 3 | `macos_dock_quit.rs` + 前端监听 | 半天 | **需 macOS 上机验证类名** |
| 4 | `platform_info.rs` + 前端类型 | 半天 | 纯增量，Linux 可先 `cargo check` |

**步骤 1-2 可以现在在 Linux 上做**（编译通过 + 单元测试），步骤 3-4 需 macOS 上机。