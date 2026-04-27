[gyroflow](https://github.com/gyroflow/gyroflow.git)

# Gyroflow Rust 借鉴项：macOS 上机实施清单

> **本文档用途**：写于 Linux 开发环境（当前没有 macOS 机器）。下次在 macOS 电脑上开发时，照着本文档逐项执行即可，不需要重新分析 Gyroflow 源码。
>
> **⚠️ 协议红线（必读）**：Gyroflow 是 **GPL-3.0** 协议，MoleStudio 是闭源商业软件（要上架 MAS）。**只能借鉴架构思路、设计模式、API 选型，严禁把 Gyroflow 的代码（哪怕改动过的）复制进 `src-tauri/`**。本文档中的所有代码骨架都是按我们自己的依赖（objc2 / objc2-foundation）重写的实现思路，不是 Gyroflow 代码。
>
> **涉及我们现有文件的落点**：
> - `src-tauri/src/lib/core/file_ops.rs`（现有文件操作 + TOCTOU 安全校验链，1473 行）
> - `src-tauri/src/lib/core/sudo.rs`（特权路径，822 行）
> - `src-tauri/src/controllers/clean.rs`（唯一一处 `std::sync::RwLock`，第 87 行）
> - `src-tauri/src/controllers/platform.rs`（平台能力命令入口）
> - `src-tauri/src/lib/platform/mod.rs`（平台模块）
> - `src-tauri/Cargo.toml`（已有 `objc2-foundation 0.3.2`，且已开启 `NSURL` feature —— 无需新增 objc 依赖）

---

## 一、macOS 沙盒安全作用域资源管理 【状态：待定 · 继续考察】

### 1.1 Gyroflow 是怎么做的

参考文件：`demo/gyroflow/src/core/filesystem/apple.rs`（207 行，很短，建议上机前通读一遍）

核心思路三件套：

1. **引用计数开关**：用 `Mutex<HashMap<String, i64>>` 记录每个 URL 被 start 了几个引用。同一个目录反复 start 只计数，不重复调用系统 API；最后一个引用释放时才真正 close。
2. **延迟关闭线程**：close 不立即执行，把"到期时间戳"存进 `AtomicUsize`，由一个后台线程每秒检查，全部过期后统一 close。原因：扫描/读取流程里 start/stop 交错高频出现，立即 close 会反复触发沙盒授权检查，性能差且可能闪断。
3. **书签持久化**：把 `CFURLBookmarkCreationOptions(1 << 11)`（即 `kCFURLBookmarkCreationWithSecurityScope`）创建的 bookmark data 用 `zlib + base91` 压缩成字符串存盘，下次启动 `resolve` 回来。resolve 时检查 `is_stale`（书签失效，比如目录被移动），失效则提示用户重新授权。

它还配套了一个 `ALLOWED_FOLDERS: RwLock<HashSet<String>>`（在 `filesystem/mod.rs`），记录"用户明确授权过的目录"，沙盒下所有文件操作先查这个集合，不在集合内直接拒绝（fail-closed）。

### 1.2 为什么对我们是"待定"——三个必须先想清楚的问题

这是我们和 Gyroflow 场景最大的差异点，**上机考察时重点验证这三个问题**：

| # | 问题 | 现状依据 | 考察动作 |
|---|------|---------|---------|
| ① | **sudo 特权路径在沙盒下不可用** | 我们有 `lib/core/sudo.rs`（822 行）做 root 删除；MAS 沙盒进程无法 setuid/起 AppleScript 提权，苹果明确要求 MAS 应用不做提权 | 真机验证：MAS 构建里 `sudo_output` 直接失败的行为是否可控、UI 能否优雅降级。与 `docs/授权方案.md`、`root权限.md`、`.trae/rules/03_硬约束与红线.md` 一起决策：MAS 版砍掉哪些 root 功能，官网版保留哪些 |
| ② | **哪些功能需要用户显式授权目录** | 清理/卸载主要扫固定路径（`~/Library/Caches` 等容器内/公共可读路径），仅"项目清理器"、自定义扫描路径需要用户选目录 | 列一份「需要 NSOpenPanel 的功能清单」，再决定书签机制的覆盖面。若只有 1-2 处，实现可以非常薄 |
| ③ | **trash crate 在沙盒下的行为** | `Cargo.toml` 注释明确写了 `trash = "3"  # MAS 合规关键` | 真机验证：沙盒内 `trash::delete` 移到废纸篓是否正常（预期可用，因为走 NSFileManager trashItem）；跨卷删除行为如何 |

### 1.3 上机考察步骤（不写代码，只验证）

```bash
# 1. 真机确认 API 存在（objc2-foundation 0.3.2，NSURL feature 已开）
#    在 src-tauri 里临时写个 #[test]：
#    NSURL::fileURLWithPath_isDirectory(...) 后调用
#    url.startAccessingSecurityScopedResource() / stopAccessingSecurityScopedResource()
#    预期：非书签路径返回 false（说明 API 可调，只是没授权）

# 2. 验证书签创建/解析（NSOpenPanel 选一个目录后）：
#    - url.bookmarkDataWithOptions_includedKeys_resourceValuesForKeys_relativeToURL_error(...)
#      options 传 NSURLBookmarkCreationOptions(1 << 11)  // withSecurityScope
#    - 下次启动用 URLByResolvingBookmarkData 解析（options 同样带 1 << 10，withSecurityScope）
#    - 记录：书签二进制大概多大、resolve 返回的 isStale 怎么拿到
#    - 若 objc2-foundation 没生成这些绑定，回退方案：加 objc2-core-foundation 用 CFURL 系 API

# 3. 验证 entitlements 生效路径：
#    tauri.conf.json 的 bundle.macOS.entitlements 目前是 null。
#    建 src-tauri/entitlements/mas.plist 试配（见 1.4），codesign 后用
#    codesign -d --entitlements - <app路径> 确认签进去了
```

### 1.4 真机通过后的实现骨架（考察通过再动手）

新文件 `src-tauri/src/lib/platform/security_scope.rs`，三部分：

```rust
// ① 引用计数表（借鉴 Gyroflow 思路，自己实现）
static OPENED: Mutex<HashMap<String, i64>> = ...;

// ② macOS 底层（cfg(target_os = "macos") 下）
fn native_start(path: &str) -> bool {
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
    unsafe { url.startAccessingSecurityScopedResource() }
}
fn native_stop(path: &str) { /* 对应 stop */ }

// ③ RAII guard，业务代码只接触这个
pub struct ScopeGuard { path: String }
impl ScopeGuard {
    pub fn acquire(path: &str) -> Option<Self> { /* 计数+1，首次调 native_start */ }
}
impl Drop for ScopeGuard {
    fn drop(&mut self) { /* 计数-1，归零后标记"待关闭"交给延迟关闭线程 */ }
}
```

书签存取放 `tauri-plugin-store`（我们已有该插件），key 约定 `scope_bookmark:<路径哈希>`。

配套 entitlements（`src-tauri/entitlements/mas.plist`，先建草稿）：

```xml
<key>com.apple.security.app-sandbox</key><true/>
<key>com.apple.security.files.user-selected.read-write</key><true/>
<key>com.apple.security.files.bookmarks.app-scope</key><true/>
```

### 1.5 待考察结论登记表（上机后填写）

- [ ] sudo 在沙盒内的失败表现：______
- [ ] 需要 NSOpenPanel 的功能清单：______
- [ ] trash crate 沙盒内行为：______
- [ ] 书签创建/resolve 是否可行、isStale 获取方式：______
- [ ] 最终决策：实现 / 继续搁置 / 砍功能，理由：______

---

## 二、统一文件系统抽象层 【状态：待实施】

### 2.1 Gyroflow 的做法（了解即可，不照搬）

参考文件：`demo/gyroflow/src/core/filesystem/mod.rs`（653 行）

它做的是**全盘 URL 化**：所有文件操作（read/write/exists/list_folder/open_file…）统一收 `url` 参数而不是 path，`FileWrapper` 实现 `Read + Seek + Drop`（Drop 时自动 stop_accessing_url），入口处统一做沙盒白名单检查（fail-closed）。文件头 25 行注释就是它的 5 条 URL 假设，值得读。

**它为什么这么做**：跨 4 个平台（macOS/Windows/Android/iOS），Android 的 `content://` 和 iOS 沙盒根本没有稳定路径。

**我们为什么不能照搬**：我们 macOS-only、全库路径导向，且 `file_ops.rs` 上挂着 TOCTOU 防护（`_mole_privileged_path_has_mutable_ancestor`）、白名单（`app_protection`）、sudo 特权删除、dry-run 注册表这一整条安全链。全盘 URL 化 = 推倒重写 + 高风险 + 零收益。

### 2.2 我们的实施方案：薄门卫层（不 URL 化，只收敛授权检查）

目标：**把"这个路径现在能不能摸"的判断收敛到一个模块**，为将来 MAS 沙盒留好插槽。分三步走，每步独立可交付：

**第 1 步：新建 `src-tauri/src/lib/platform/fs_gate.rs`**

```rust
/// 路径访问守门员。
/// 官网版(full)：恒放行（返回 Some(())）。
/// MAS 版(mas)：检查路径是否属于——
///   a) 应用容器内            → 放行
///   b) 用户授权过的目录(书签) → 放行
///   c) 系统公开只读路径       → 放行（扫描元数据）
///   d) 其他                  → 拒绝（Err，含给前端的降级提示 key）

pub fn check_writable(path: &str) -> Result<(), GateError> { ... }
pub fn check_scannable(path: &str) -> Result<(), GateError> { ... }
```

**第 2 步：在现有模块入口处挂接（不动内部逻辑）**

挂接点按优先级：
1. `file_ops::safe_remove` / `safe_sudo_remove` 系列（写操作，最关键）
2. `controllers/clean.rs` 的扫描入口
3. `lib/clean/project.rs`（项目清理器——第一个接 NSOpenPanel 的功能）
4. `lib/uninstall/batch.rs`

方式：入口函数开头加一行 `fs_gate::check_writable(path)?;`。这样将来沙盒开关只改 `fs_gate` 一个文件。

**第 3 步：与第一节的安全作用域 guard 组装**

`fs_gate` 内部对"授权目录"的路径自动 `ScopeGuard::acquire`（第一节考察通过后实现；考察不通过，`mas` feature 下 b 类直接拒绝）。

### 2.3 验收标准（macOS 上）

- [ ] 官网版（full feature）：全功能回归通过，`fs_gate` 行为与改造前完全一致（no-op）
- [ ] `pnpm tauri:dev:mas`（mas feature）能编译、能启动
- [ ] MAS 版里未授权路径的删除请求被拒且前端拿到可读的错误（走 `events.rs` / 前端 `src/types/mole.ts` 的错误约定）
- [ ] `file_ops.rs` 既有 TOCTOU/白名单测试全绿（`src-tauri/tests/` 下已有 3 个测试文件）

---

## 三、Store Package 检测（运行时沙盒识别）【状态：待实施 · 最简单，建议先做】

### 3.1 Gyroflow 的做法

参考文件：`demo/gyroflow/src/util.rs` 第 506-522 行（`is_store_package`）和 `demo/gyroflow/src/core/filesystem/mod.rs` 第 644-652 行（`is_sandboxed`）

思路：**编译期 + 运行时双通道**。
- macOS 沙盒应用运行时进程环境里有 `APP_SANDBOX_CONTAINER_ID` 环境变量，非沙盒进程没有 → `std::env::var("APP_SANDBOX_CONTAINER_ID").is_ok()` 即可判定，无需任何 Apple API。
- Gyroflow 的规则：只有"沙盒化的 mac 应用"才算商店包（它官网版不沙盒）。

### 3.2 我们的实施方案

我们已经有**编译期**双版本（`full` / `mas` feature，见 `Cargo.toml` 第 10-14 行），缺的是**运行时**判定（官网版也可能被装进沙盒环境跑，或用户手动改 entitlements）。两个都上，编译期为主、运行时兜底：

```rust
// src-tauri/src/lib/platform/mod.rs（或新建 runtime_env.rs）

/// 运行时检测：进程是否处于 App 沙盒内
#[cfg(target_os = "macos")]
pub fn is_sandboxed() -> bool {
    std::env::var("APP_SANDBOX_CONTAINER_ID").is_ok()
}

/// 综合判定：是否商店精简版
/// 编译期 mas feature 说了算；非 mas 构建下若检测到沙盒也视为受限环境
pub fn is_store_package() -> bool {
    #[cfg(feature = "mas")]
    { return true; }
    #[cfg(not(feature = "mas"))]
    {
        #[cfg(target_os = "macos")]
        { return is_sandboxed(); }
        #[cfg(not(target_os = "macos"))]
        { return false; }
    }
}
```

暴露给前端（与现有契约对齐，见 `.trae/rules/01_项目身份与技术栈.md` 前后端契约节）：

```rust
// src-tauri/src/controllers/platform.rs 里加 command
#[tauri::command]
pub fn get_runtime_env() -> RuntimeEnv {
    RuntimeEnv {
        store_package: is_store_package(),
        sandboxed: is_sandboxed(),   // macOS 下才有
        full_build: cfg!(feature = "full"),
    }
}
// → 前端在 src/constants/tauri-commands.ts 登记命令名
// → 前端在 src/types/mole.ts 补 RuntimeEnv 类型
// → UI 据此对 MAS 版做功能降级展示（对应 ui 文档的功能矩阵）
```

### 3.3 验证步骤（macOS 上，10 分钟）

```bash
# ① 官网版（非沙盒）：直接跑 pnpm tauri dev
#    前端 console 打印 get_runtime_env() → 预期 sandboxed=false
#    终端验证：APP_SANDBOX_CONTAINER_ID 未出现在进程环境
#      ps eww <pid> | grep -c APP_SANDBOX  → 0

# ② 沙盒版：临时给 tauri.conf.json 配上带 com.apple.security.app-sandbox 的
#    entitlements plist，build 后运行
#    → 预期 sandboxed=true
#    且 codesign -d --entitlements - target/release/bundle/macos/*.app 可见该 key

# ③ 编译期开关：cargo build --features mas 后二进制内 is_store_package 恒 true
#    （可在 dev 日志里打印一次确认）
```

---

## 四、parking_lot 替换 std::sync 锁 【状态：待定 · 继续考察】

### 4.1 Gyroflow 的做法

全库统一用 `parking_lot::RwLock / Mutex`（见 `demo/gyroflow/src/core/lib.rs` 的 `StabilizationManager`，一整套 `Arc<RwLock<...>>`）。收益：不中毒（无 poison 语义，`write()` 直接返回 guard，不用 `map_err`）、无 futex 重活时更快、API 更干净。

### 4.2 我们现状（已核实）

全库**只有一处**锁：`src-tauri/src/controllers/clean.rs:87`

```rust
static SCAN_REGISTRY: std::sync::RwLock<Option<ScanSnapshot>> = std::sync::RwLock::new(None);
```

（第 91-100 行的 `store_scan_snapshot` / `take_scan_snapshot` 用的就是 std API 的 `write()`，换 parking_lot 后这两处 `.map_err(|_| "lock poisoned")` 可以直接删掉。）

### 4.3 考察结论（为什么待定）

单点使用，替换收益接近 0，但引入依赖有维护成本。**上机考察两件事再决定**：

```bash
# ① 确认编译成本：parking_lot 是否已是传递依赖（是的话编译成本≈0）
cargo tree -p parking_lot   # 在 src-tauri/ 下执行

# ② 锁竞争画像：analyze/clean 扫描是 rayon 并发递归（Cargo.toml 注释写明
#    "对齐 Go scanner.go 的 goroutine 池"），未来若 scan 结果上报与
#    SCAN_REGISTRY 读写产生热点，才值得换。
#    简单验证：大目录扫描时 sudo dtrace / Instruments Time Profiler 看
#    _pthread_cond_wait 是否出现在热点栈。
```

### 4.4 决策规则（写死，免得再讨论）

- 当前：**不引入**，保持 std。
- 触发条件（满足任一才引入）：
  a) `cargo tree` 显示 parking_lot 已是直接传递依赖且无版本冲突；
  b) 真机 profile 出现锁热点（SCAN_REGISTRY 或未来新增的共享扫描缓存）；
  c) 项目内锁数量 ≥ 3 处（poison 处理代码开始重复）。
- 届时改法：`Cargo.toml` 加 `parking_lot = "0.12"`；只改 `clean.rs` 一处 `use std::sync::RwLock` → `use parking_lot::RwLock`，删除 100 行附近的 `map_err` 中毒处理。

---

## 五、keep-awake 防止系统休眠 【状态：待实施 · 实用小件】

### 5.1 Gyroflow 的做法

参考文件：`demo/gyroflow/src/rendering/mod.rs` 第 207-210 行

```rust
let _prevent_system_sleep = keep_awake::inhibit_system("Gyroflow", "Rendering video");
```

关键在 **RAII**：guard 变量持有期间系统不休眠，函数返回（或提前 `?` 出错）guard drop 自动恢复。它用的是作者自己的 fork（AdrianEddy/keep-awake-rs），我们用 crates.io 标准版即可，**不要用它的 fork**（协议同样 GPL 风险 + 我们用不到它的补丁）。

### 5.2 我们的实施方案

**为什么需要**：全盘扫描（analyze）、深度清理（clean）、批量卸载（uninstall batch）都可能跑几十分钟；macOS 默认空闲几分钟就 AppNap/休眠，进度会停滞、甚至网络类任务（update 检测）断掉。

**选型**：crates.io 的 `keep-awake`（macOS 下走 IOKit IOPMAssertion，MAS 沙盒内可用；Windows/Linux 也有实现，方便将来跨平台）。备选：自己用 objc2 调 `IOPMAssertionCreateWithName`（约 30 行），若 crates.io 版本和 objc2 0.6 冲突再走备选。

**接入点（RAII，包住整个任务生命周期）**：

```rust
// src-tauri/src/lib/core/keep_awake.rs（新文件，薄包装便于统一打点）
pub struct MoleKeepAwake { _guard: keep_awake::KeepAwake }

pub fn acquire(reason: &str) -> anyhow::Result<MoleKeepAwake> {
    let guard = keep_awake::Builder::default()
        .app_name("MoleStudio")
        .reason(reason)          // 如 "Deep cleaning" / "Disk analysis"
        .start()?;
    Ok(MoleKeepAwake { _guard: guard })
}
```

| 接入位置 | reason 文案 |
|---------|------------|
| `controllers/analyze.rs` 全盘扫描入口 | "Disk analysis" |
| `controllers/clean.rs` apply 阶段 | "Deep cleaning" |
| `lib/uninstall/batch.rs` 批量卸载 | "Uninstalling apps" |

**注意**：guard 必须在 Tauri command 的 async 任务内持有到任务结束；不要在 command 一进来就 acquire、函数结束就 drop 了事——扫描如果是 spawn 出去的长任务，guard 要 move 进那个任务闭包。

### 5.3 验证步骤（macOS 上）

```bash
# ① 运行一次全盘扫描，另一个终端观察断言：
pmset -g assertions
#    预期出现：PreventUserIdleSystemSleep=1，Details 里 app 名为 MoleStudio / reason 为我们的文案

# ② 扫描结束（或中途取消）后再查：
#    预期断言消失（RAII drop 生效）

# ③ 系统设置 → 节能里把睡眠时间调到最短，实测长时间任务不被打断

# ④ MAS 构建下重复 ①（确认 IOKit assertion 在沙盒内可用）
```

---

## 六、上机总执行顺序（建议）

按依赖关系与风险排序，从便宜到贵：

1. **第三节 Store Package 检测** —— 半天，纯增量，先拿到 `get_runtime_env` 这块地基
2. **第五节 keep-awake** —— 半天，独立无依赖，立刻提升体验
3. **第二节 fs_gate 第 1、2 步** —— 1 天，官网版下是 no-op，先铺好插槽
4. **第一节沙盒考察（只验证不实现）** —— 半天到一天，按 1.3 步骤跑，填 1.5 登记表
5. **第一节沙盒实现 + 第二节第 3 步** —— 视第 4 步结论再定
6. **第四节 parking_lot** —— 触发条件满足才动

## 七、风险速查

| 风险 | 说明 | 缓解 |
|------|------|------|
| GPL 传染 | 复制 Gyflow 代码进闭源项目 | 只读思路，本文档代码骨架均为重写思路 |
| sudo × 沙盒冲突 | MAS 版无法提权 | 先决策功能矩阵再动 entitlements |
| entitlements 改动影响签名 | 改 plist 后需重新签 + 真机验证 | 每次只加一个 key，codesign 确认 |
| keep-awake crate 与 objc2 版本冲突 | 若 crates.io 版本拉入旧 objc2 | 回退方案：objc2 + IOKit 手写 30 行 |
| fs_gate 误伤官网版 | check 挂错位置导致 full 版行为变化 | full feature 下 no-op + 现有测试全绿为准 |


[floral-notepaper](https://github.com/Achilng/floral-notepaper.git)

# 花笺（floral-notepaper）借鉴项：决策与实施清单

> **协议说明**：花笺是 **MIT** 协议（比 Gyroflow 的 GPL 宽松得多），代码可参考、可复制改造。本文档代码骨架仍按 MoleStudio 双版本构建（`full` / `mas` feature）重写思路给出，便于直接落地。
>
> **本次分析结论总表**（其余 11 个借鉴点未采纳，速查见文末附表）：

| 花笺借鉴点 | 决策 | 状态 |
|-----------|------|------|
| ① 自实现完整更新器系统 | 上线阶段做，本版本搁置，本文留档 | 待实施（上线前） |
| ② PlatformInfo 安装形态检测 | **现在提取布局**，避免未来写兼容代码 | 本版本实施 |

---

## 一、自实现完整更新器系统 【状态：上线阶段实施 · 本版本搁置】

### 1.1 决策记录

- **本版本不做**。理由：MAS 版本来就不能自更新（必须走 App Store 更新通道），官网直发版的更新源（GitHub Releases / Mirror酱）也要上线前才能定，现在做是空转。
- **触发时机**：进入上线准备阶段（打包流程定型、更新源确定）后，按 1.5 的顺序实施。
- **本文档作用**：上线时不用重新分析源码，照本节实施。两套参考实现均已深度分析：
  - 花笺 `demo/floral-notepaper/src-tauri/src/updater/`（MIT，可直接翻）
  - Zed `demo/zed/crates/auto_update/src/auto_update.rs`（Apache-2.0，1877 行）

### 1.2 两套参考实现对比分析

> **分析结论：推荐混合方案** — 以 Zed 的简洁架构为主干，补上花笺的两个关键安全点（ditto + 最简回滚），预估 **300-400 行核心代码**（vs Zed 100 行 / 花笺 4000+ 行 + helper 二进制）。

#### 1.2.1 Zed 方案（Apache-2.0，~100 行核心）

```
下载 DMG → hdiutil attach -nobrowse -mountroot → rsync -av --delete → detach -force
无回滚 / 无中间暂存 / 直接覆盖运行中的 .app
```

| 优点 | 说明 |
|------|------|
| 极简 | 不需要 helper 二进制，主进程内异步执行即可 |
| `MacOsUnmounter` RAII | Drop 自动卸载 DMG，`unmount()` 走 happy path，Drop 走 safety net |
| 唤醒智能 | `restart_after_wake`：下载/检查中→重启；**安装中（rsync）→不中断** |
| 残留清理 | `cleanup_stale_installer_dirs`：扫描 temp_dir 下 >24h 的旧安装目录 |
| 依赖检查 | `which::which("rsync")` 启动时确认 |
| 进度回调 | 流式下载 8KB buffer + 百分比变化时才通知 UI（避免刷屏） |

| 缺点 | 说明 |
|------|------|
| rsync 不保留 macOS 元数据 | resource fork、扩展属性可能丢失，影响代码签名和 Gatekeeper |
| **无回滚** | rsync 中途崩溃 → .app 损坏 → 用户只能手动重新下载 |
| rsync 非系统保证 | macOS 自带但 Apple 可能在未来移除（如已移除的 Python/Ruby） |
| 直接覆盖运行中 .app | 虽然进程已加载到内存不受影响，但如果中途断电则无法恢复 |

#### 1.2.2 花笺方案（MIT，~4000 行 + helper 二进制）

```
下载 DMG → hdiutil attach -readonly → ditto 暂存 → verify → swap(rename旧+移入新)
→ 尝试重启 → 失败则 rollback
完整回滚 / 中间暂存 / 文件锁 / 残留清理 / 独立 helper 进程
```

| 优点 | 说明 |
|------|------|
| `ditto` 替代 rsync | Apple 官方推荐工具，完整保留 resource fork、扩展属性、代码签名 |
| 完整回滚 | `MacosRollbackPlan`：旧 .app rename 为 backup → 新版启动失败自动还原 |
| Bundle 验证 | `verify_macos_bundle`：暂存后先验证 .app 结构完整性再替换 |
| DMG 卸载稳健 | 先 `detach`，失败再 `detach -force`（两级重试） |
| 独立 helper | 主进程完全退出后才替换，避免文件占用 |

| 缺点 | 说明 |
|------|------|
| 复杂度高 | 4000+ 行 + 独立 helper 二进制（需额外签名、打包、维护） |
| 文件锁 | 增加了 `.lock` + pid + stale 检测（5 分钟超时） |
| 过度工程 | 对于单窗口 Tauri 应用，多窗口协调、watchdog 等机制用不上 |

#### 1.2.3 关键差异对照表

| 维度 | Zed | 花笺 | 对清理软件用户的影响 |
|------|-----|------|---------------------|
| 代码量 | ~100 行核心 | ~4000 行 + helper | 维护成本差 40 倍 |
| 覆盖方式 | `rsync -av --delete` 直接覆盖 | `ditto` 暂存 → `rename` 原子交换 | Zed 中途崩溃 .app 可能损坏 |
| macOS 元数据 | rsync 不保证 resource fork | `ditto` 完整保留 | 代码签名 / Gatekeeper 风险 |
| 回滚 | **无** | **有**（完整 MacosRollbackPlan） | 清理软件更新失败 = 用户信任危机 |
| Helper 二进制 | macOS **不需要** | 需要独立 helper 进程 | 少一个二进制 = 少一份签名维护 |
| 依赖 | rsync（macOS 自带但非保证） | ditto（macOS 内置，Apple 官方推荐） | ditto 更可靠 |
| DMG 卸载 | 直接 `detach -force` | 先 `detach`，失败再 `-force` | 花笺更稳健 |
| 唤醒处理 | 智能区分阶段 | 未特别处理 | Zed 这点更优 |

### 1.3 推荐方案：混合架构（Zed 骨架 + ditto + 最简回滚）

**设计原则**：用 Zed 的简洁流程 + ditto 替换 rsync + rename-based 最简回滚。

```
推荐流程（~300-400 行核心代码）：

1. 检查更新  →  Idle → Checking（轮询 60min，参考 Zed POLL_INTERVAL）
2. 下载 DMG  →  Checking → Downloading（流式 8KB buffer + 进度回调）
3. 安装      →  Downloading → Installing：
   a. hdiutil attach -nobrowse -mountroot <temp_dir>    ← Zed 模式
   b. ditto <mounted.app> <staged.app>                   ← 花笺模式（保留元数据）
   c. hdiutil detach <mount>，失败 detach -force          ← 花笺两级重试
   d. rename <running.app> → <running.app>.backup         ← 最简回滚
   e. rename <staged.app> → <running.app>                 ← 原子交换
   f. 成功：删除 .backup；失败：从 .backup 还原            ← 最简回滚
4. 清理      →  cleanup_stale_installer_dirs（>24h）      ← Zed 模式
5. 唤醒      →  下载/检查中→重启检查；安装中→不中断         ← Zed 模式
```

**采纳 Zed 的：**
1. 不需要 helper 二进制 — 在主进程内 `spawn_blocking` 执行
2. `hdiutil attach -nobrowse -mountroot` — 和 Zed 一样
3. `MacOsUnmounter` RAII — Drop 自动卸载
4. `cleanup_stale_installer_dirs`（>24h）
5. 系统唤醒智能重启逻辑
6. 状态机：Idle → Checking → Downloading → Installing → Updated
7. 进度回调：百分比变化时才通知 UI

**替换 Zed 的：**
1. **rsync → ditto**：`ditto` 是 macOS 原生工具，保留代码签名和 resource fork
2. **直接覆盖 → 暂存+交换**：先 ditto 暂存 → rename 旧 .app 为 .backup → 移入新 .app

**补上花笺的（精简版）：**
1. **最简回滚**：rename 旧 .app 为 `.backup`，启动失败时还原
2. **不需要**独立 helper 二进制（Zed 证明了不需要）
3. **不需要**文件锁（Tauri 单实例已有保证）
4. **DMG 两级卸载**：先 `detach`，失败再 `detach -force`
5. 下载域名白名单 + SHA-256 校验

### 1.4 参考实现拆解

#### 花笺模块拆解（12 个文件，共 12484 行，含约 1800 行测试）— 取其安全流程

| 模块 | 行数 | 职责 | 我们采纳的部分 |
|------|------|------|--------------|
| `check.rs` | 1562 | 双源检查：`UpdateCheckProvider` trait + MirrorChyan / Github 两实现 | trait 抽象让加第三个更新源零侵入 |
| `download.rs` | 1684 | 流式下载：`AtomicBool` 取消、`.part` 临时文件、重试（1s/3s/8s）、**下载域名白名单** | 域名白名单 + SHA-256 校验 |
| `helper.rs` | 4013 | DMG 挂载/卸载/回滚全流程 | **仅取 ditto + 回滚思路**，不取 helper 二进制 |
| `mod.rs` | 760 | `UpdaterState`、`ActiveTaskGuard` RAII | ActiveTaskGuard 模式可给 `SCAN_REGISTRY` |
| `commands.rs` | 732 | `install_prepare` 多窗口协调 | 单窗口暂不需要 |
| `cdk_store.rs` | 84 | `keyring` 存钥匙串 | **可提前做**：存 license |
| 其余 | ~3200 | 调度器、文件锁、状态持久化等 | 按需参考 |

#### Zed 模块拆解（auto_update.rs 1877 行）— 取其简洁架构

| 功能块 | 行范围 | 职责 | 我们采纳的部分 |
|--------|--------|------|--------------|
| 状态机 + 轮询 | 180-560 | `AutoUpdateStatus` 枚举 + `poll()` + `start_polling()` | 完整状态机 + 60min 轮询 |
| `MacOsUnmounter` RAII | 193-243 | Drop 自动卸载 DMG + `unmount_disk_image` | RAII 卸载模式 |
| `InstallerDir` | 367-408 | `tempfile::TempDir` 包装 + 自动清理 | 临时目录管理 |
| 唤醒订阅 | 447-481 | `on_system_wake` → 智能重启检查 | 唤醒处理逻辑 |
| 版本比较 | 857-957 | semver 比较 + nightly commit sha 比较 | 仅取 semver 比较 |
| `install_release_macos` | 1186-1240 | hdiutil + rsync + unmount | **架构照搬，rsync 换 ditto** |
| `download_release` | 1065-1116 | 流式下载 + Content-Length + 百分比节流 | 完整采纳 |
| `cleanup_stale` | 1242-1286 | 清理 >24h 旧安装目录 | 完整采纳 |
| 依赖检查 | 893-910 | `which::which("rsync")` | 改为检查 `ditto` |

### 1.5 MoleStudio 适配设计（上线时按此落位）

```text
src-tauri/src/lib/self_update/      ← 注意命名！
    mod.rs          UpdaterState（状态机：Idle/Checking/Downloading/Installing/Updated/Errored）
                    + MacOsUnmounter RAII + cleanup_stale_installer_dirs
    check.rs        provider trait + 实现（源待定：GitHub Releases / Mirror酱）
    download.rs     流式下载 + AtomicBool 取消 + 域名白名单 + SHA-256
    install.rs      核心安装流程：hdiutil attach → ditto 暂存 → detach（两级）
                    → rename 旧 .app 为 .backup → 移入新 .app → 验证 → 删除/回滚
    scheduler.rs    自动检查（60min 轮询 + 唤醒重启）
    platform.rs     → 直接复用第二节的 platform_info.rs，不单独建
```

**为什么叫 `self_update` 不叫 `updates`**：`src-tauri/src/lib/updates/` 已存在，且含义完全不同——它管理**其他应用**的更新检测（Sparkle appcast / iTunes / brew，`controllers/updates.rs`）。自更新是"MoleStudio 更新自己"，混用必出歧义。

**双版本门控**：

```rust
// self_update 只存在于 full 构建；mas 构建整个模块编译期剔除
#[cfg(feature = "full")]
pub mod self_update;

// 初始化处（lib.rs setup）同样 #[cfg(feature = "full")] 门控
// 前端 get_runtime_env().channel === "macAppStore" 时 UI 不渲染"检查更新"入口
```

**依赖清单（上线阶段加进 Cargo.toml，当前全没有）**：`reqwest`（blocking client 足够）、`sha2`、`semver`、`keyring`（可提前加，见 1.6）、`tempfile`（已有）。**不需要** helper `[[bin]]` 目标。

**核心安装流程伪代码**：

```rust
async fn install_release_macos(
    temp_dir: &InstallerDir,
    downloaded_dmg: &Path,
    running_app_path: PathBuf,
) -> Result<()> {
    let mount_path = temp_dir.path().join("MoleStudio");
    let staged_app = temp_dir.path().join("MoleStudio.app");
    let backup_path = running_app_path.with_extension("app.backup");

    // 1. 挂载 DMG（Zed 模式）
    new_command("hdiutil")
        .args(["attach", "-nobrowse"])
        .arg(downloaded_dmg)
        .arg("-mountroot").arg(temp_dir.path())
        .output().await?;
    let unmounter = MacOsUnmounter::new(mount_path.clone());

    // 2. ditto 暂存（花笺模式 — 保留 macOS 元数据 + 代码签名）
    let mounted_app = find_app_bundle(&mount_path)?;
    new_command("ditto")
        .arg(&mounted_app).arg(&staged_app)
        .output().await?;

    // 3. 卸载 DMG（花笺两级重试）
    unmounter.unmount().await;  // 先 detach，失败 detach -force

    // 4. 原子交换 + 最简回滚
    fs::rename(&running_app_path, &backup_path)?;       // 旧 .app → .backup
    match fs::rename(&staged_app, &running_app_path) {   // 新 .app → 目标位置
        Ok(()) => { fs::remove_dir_all(&backup_path).ok(); Ok(()) }
        Err(e) => {
            // 回滚：还原 .backup
            fs::rename(&backup_path, &running_app_path).ok();
            Err(e.into())
        }
    }
}
```

**与现有安全链的衔接**：下载的 .app 替换目标在 `/Applications`，普通用户通常无写权限 → 复用 `lib/core/sudo.rs` 特权通道，或采用花笺的"用户目录级联"策略（装在 `~/Applications` 则免提权）。上线前真机确认。

### 1.6 上线阶段执行顺序

1. **可提前的小件（本版本就能顺手做）**：
   - `keyring` 存 license/激活信息（84 行封一个 `license_store.rs`，与更新解耦）
   - `json_io` 原子写入（花笺 50 行：tmp + sync_all + rename + sync_parent_dir），给现有配置文件上保险
   - `tempfile::NamedTempFile` 原子写入（Zed 模式：`new_in(parent)` + `persist(path)`），更简洁
2. `mod.rs` 骨架：状态机 + `MacOsUnmounter` RAII + `cleanup_stale_installer_dirs`
3. `install.rs` 核心流程：hdiutil + ditto + 原子交换 + 最简回滚
4. `check.rs` / `download.rs`（先单源，Mirror酱在国内分发场景下优先级高于 GitHub）
5. `install_prepare`（衔接 `SCAN_REGISTRY`：有进行中扫描则等待/中止）
6. `scheduler.rs` + 设置项 + 唤醒订阅
7. 全链路真机测试：正常升级 / 下载中断恢复 / sha256 不匹配 / 磁盘满 / **回滚验证**（断电模拟）

### 1.7 风险速查

| 风险 | 说明 | 缓解 |
|------|------|------|
| 与 `lib/updates/` 混淆 | 语义完全不同的两套"更新" | 强制 `self_update` 命名，文档注释互相指路 |
| /Applications 写权限 | 替换 .app 需要特权 | 上线前真机决策：sudo 通道 vs 建议 ~/Applications |
| 下载源供应链 | manifest 指向恶意 URL | 花笺的域名白名单照抄；SHA-256 必验 |
| mas 构建剔除不净 | cfg 漏门控导致 MAS 审核被拒 | `cargo build --features mas` 后 grep 二进制确认无 self_update 符号 |
| ditto 不存在 | 极小概率（macOS 内置命令） | 启动时 `which::which("ditto")` 检查 |
| 回滚不完整 | .backup 还原后数据文件可能已迁移 | 更新前检查数据文件格式版本；回滚只还原 .app，不动用户数据 |
| rename 跨文件系统 | 如果 .app 和 temp 在不同卷，`rename` 失败 | ditto 暂存目录必须在同一卷（用 `temp_dir` 在 app 同级目录创建） |

---

## 二、PlatformInfo 安装形态检测 【状态：本版本实施 · 布局提取】

### 2.1 决策记录

- **MAS 上架推迟**：等上线几个版本成熟后再考虑，本版本不发布 MAS。
- **但布局现在就提取**：用户原话"不想以后来做兼容代码"。含义是——`DistributionChannel`（分发渠道）必须现在成为架构里的一等公民，未来所有功能分支（sudo 降级、trash 行为、自更新开关、entitlements 差异）都**只查这一个统一出口**，而不是将来在每个功能里散落 `is_store_package()` 判断。
- 本节是**第三节（Store Package 检测）的升级实施版**：第三节规划的 `RuntimeEnv{store_package, sandboxed, full_build}` 升级为完整 `PlatformInfo` 模型，第三节代码骨架作废，以本节为准。

### 2.2 花笺参照点（MIT，可直接借）

- `PlatformInfo` 结构体 + **`OnceLock` 进程级缓存**：os/arch/install_kind 全是进程生命周期内不变量，只算一次（花笺注释：Windows 下可省最多 4 次 `reg query` 子进程；我们 macOS 下收益是统一出口 + 零重复计算）。
- `find_macos_app_bundle`：从 exe 向上逐级找 `.app` 包，10 行。
- debug 模式处理：`target/debug` 下 install_kind 是 `Unknown`，但 `ensure_in_app_updates_supported` 对 debug 放行（dev 环境可测完整流程）。

### 2.3 实施方案（替代第三节骨架）

新文件 `src-tauri/src/lib/platform/platform_info.rs`：

```rust
use serde::Serialize;
use std::{env, path::PathBuf, sync::OnceLock};

/// 分发渠道。未来加新渠道（如 Homebrew cask）只扩这里 + detect_channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DistributionChannel {
    /// 官网直发（.dmg / zip）
    Direct,
    /// Mac App Store（沙盒）
    MacAppStore,
}

/// 运行形态：正式 .app 包内 / 开发模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    MacosAppBundle,
    Unknown, // target/debug 或找不到 .app
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub channel: DistributionChannel,
    pub install_kind: InstallKind,
    pub app_version: String,
    pub current_app_bundle: Option<String>,
    pub sandboxed: bool,   // 运行时（第三节逻辑）
    pub full_build: bool,  // 编译期 cfg!(feature = "full")
}

static PLATFORM_CACHE: OnceLock<PlatformInfo> = OnceLock::new();

/// 统一出口：全库所有"渠道分支"只准查这里
pub fn current() -> &'static PlatformInfo {
    PLATFORM_CACHE.get_or_init(compute)
}

fn compute() -> PlatformInfo {
    let exe = env::current_exe().ok();
    let bundle = exe.as_ref().and_then(find_macos_app_bundle);
    let sandboxed = is_sandboxed();
    PlatformInfo {
        channel: detect_channel(sandboxed),
        install_kind: if bundle.is_some() { InstallKind::MacosAppBundle }
                      else { InstallKind::Unknown },
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        current_app_bundle: bundle.map(|p| p.to_string_lossy().to_string()),
        sandboxed,
        full_build: cfg!(feature = "full"),
    }
}

#[cfg(target_os = "macos")]
fn is_sandboxed() -> bool { env::var("APP_SANDBOX_CONTAINER_ID").is_ok() }
#[cfg(not(target_os = "macos"))]
fn is_sandboxed() -> bool { false }

/// 编译期 mas 说了算；非 mas 构建下检测到沙盒视为受限兜底（继承第三节规则）
fn detect_channel(sandboxed: bool) -> DistributionChannel {
    #[cfg(feature = "mas")]
    { let _ = sandboxed; DistributionChannel::MacAppStore }
    #[cfg(not(feature = "mas"))]
    { if sandboxed { DistributionChannel::MacAppStore } else { DistributionChannel::Direct } }
}

/// 花笺思路（MIT）：exe 向上找 .app 包
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

`lib/platform/mod.rs` 挂 `pub mod platform_info;`。

命令层（升级第三节规划的 `get_runtime_env`，`controllers/platform.rs`）：

```rust
#[tauri::command]
pub fn get_runtime_env() -> &'static PlatformInfo {
    crate::lib::platform::platform_info::current()
}
// 注意：返回 &'static 无需 clone；serde 直接序列化 camelCase
```

前端契约（三步，对齐 `.trae/rules/01_项目身份与技术栈.md` 契约节）：
1. `src/constants/tauri-commands.ts` 登记 `get_runtime_env`
2. `src/types/mole.ts` 补 `PlatformInfo / DistributionChannel / InstallKind` 类型
3. 消费规则（**这是"不写兼容代码"的关键，写进规范**）：
   - 任何功能要做渠道差异，只准 `channel === "macAppStore"` 分支，禁止再散落 `full_build / sandboxed` 的裸判断（它们仅供诊断展示）
   - 例：sudo 特权功能 → `channel === "direct"` 才启用；未来自更新 → `channel === "direct"` 才初始化（对应第一节 1.3）

### 2.4 验证步骤

```bash
# Linux（现在就能做）
cd src-tauri && cargo check                      # 非 macOS 编译通过（cfg 分支）
cargo check --features mas                       # mas feature 下编译通过
cargo test -p molestudio platform_info           # 单测：detect_channel / find_macos_app_bundle

# macOS 上机（5 分钟）
# ① 官网版 dev：前端 console 打印 get_runtime_env() → 预期
#    { channel:"direct", installKind:"unknown", sandboxed:false, fullBuild:true }
#    （dev 下 installKind=unknown 属预期，见 2.2 debug 处理）
# ② 打包后运行 bundle/macos/*.app → installKind 变 "macosAppBundle"
# ③ 临时配沙盒 entitlements 构建 → channel 变 "macAppStore"（验证运行时兜底）
# ④ cargo build --features mas → channel 恒 "macAppStore"（编译期优先）
```

### 2.5 风险速查

| 风险 | 说明 | 缓解 |
|------|------|------|
| OnceLock 缓存时机 | 若未来出现"启动后才变化"的判定源（目前没有），缓存会过期 | compute 只放进程不变量；可变量走独立 API |
| 渠道判断散落回潮 | 新代码又写裸 `cfg!(feature="mas")` | 代码评审规则 + `Grep "cfg!(feature"` 应只出现在 platform_info.rs |
| 与第三节文档冲突 | 第三节骨架已作废 | 第三节开头加一行"已被本文档第二节取代"（实施时顺手改） |

---

## 附：花笺其余借鉴点速查（本次均未采纳，需要时再翻分析记录）

| 点 | 一句话结论 |
|----|-----------|
| keyring 钥匙串 | 采纳了"上线前提前做"（第一节 1.4 第 1 步） |
| ActiveTaskGuard RAII + poison 恢复 | 花笺模式留档，`SCAN_REGISTRY` 若出锁热点时参照 |
| install_prepare 多窗口协调 | 单窗口版本暂不需要，多窗口化时再借 |
| json_io 原子写入 | 顺手件，第一节 1.4 第 1 步一并做 |
| trash v3→v5 | 不动，MAS 合规注释已明确用 v3，升级等有实际动机 |
| macOS 全屏退出动画处理 | 有全屏需求时再借（desktop.rs 1208-1265） |
| single-instance / autostart / 纯 Rust i18n / config-data 分离 | 低价值，按需 |

[zed](https://github.com/zed-industries/zed.git)

[dbx](https://github.com/t8y2/dbx.git)

# dbx 借鉴项：Dock 退出拦截（macOS）【状态：待实施 · 高优先级】

> **协议说明**：dbx 为 **Apache-2.0**（宽松协议，同 Zed）。本文代码骨架为按 MoleStudio 现有依赖（objc2 / objc2-app-kit）重写的实现思路，可直接落地。
>
> **参考文件**：`demo/dbx/src-tauri/src/macos_app_delegate.rs`（65 行，建议实施前通读）

## 一、决策记录

- **本版本实施**。理由有三：
  1. **Dock 退出是当前唯一的真实退出路径**：主窗口关闭走"隐藏到托盘"（`lib.rs:155-160` prevent_close + hide，保 sudo keepalive），托盘目前**没有菜单**（`tray.rs` 仅左键弹 dashboard，无退出项）→ 用户想真正退出应用，只有 Dock 右键 → 退出这一条路。
  2. **这条路现在是"裸奔"的**：Dock 退出走 AppKit 的 `applicationShouldTerminate:`，完全绕过 Tauri 的 `WindowEvent::CloseRequested`（我们只拦了窗口红绿灯）→ 全盘扫描跑到 15 分钟、批量卸载执行到一半，Dock 退出会**立即杀进程**，无任何确认。
  3. 清理软件的长任务（analyze/clean/uninstall/optimize）是核心场景，中断保护是基本体验。

## 二、dbx 的做法（原理拆解）

Tauri 底层用 tao 管理 NSApplication。tao 注册的 AppDelegate 类（`TaoAppDelegateParent`）**没有实现** `applicationShouldTerminate:`，导致 Dock 退出绕过一切 Tauri 事件直接终止进程。dbx 的修复分三步：

```
1. class_addMethod 给 TaoAppDelegateParent 动态注入 applicationShouldTerminate:
2. 回调里查 confirmed_exit 标志：
   - true  → 返回 TerminateNow（放行，正常退出）
   - false → 触发确认流程 + 返回 TerminateCancel（取消本次终止）
3. 前端确认后 → allow_next_exit() 置标志 + app.exit(0) → 再次触发 terminate → 放行
```

关键细节（直接照抄的工程判断）：
- `OnceLock<AppHandle>` 存句柄 + `Once` 保证只注入一次；
- 注入失败（类已实现该方法）只 `eprintln!` 告警，不 panic——降级为现有行为（裸退出）；
- `extern "C-unwind"` 签名 + `Imp` transmute，方法类型编码 `Q@:@`（返回 NSUInteger）。

## 三、实施方案

### 3.1 前置件：AppBusyState 忙碌计数（新文件，~30 行）

现状盘点：clean 有 `SCAN_REGISTRY`（快照存取，非忙碌标志）、scanner 有 `SCAN_ESTIMATED_TOTAL_FILES`（进度），**没有一个统一的"有任务在跑"标志**。Dock 拦截需要一个。最小方案——原子计数器，长任务进入 +1 退出 -1：

```rust
// src-tauri/src/lib/core/busy_state.rs
use std::sync::atomic::{AtomicUsize, Ordering};

static BUSY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 长任务入口调用（analyze 扫描 / clean apply / uninstall batch / optimize 执行）。
/// 返回 RAII guard，任务结束（含 early return / panic）自动 -1。
pub fn enter_busy() -> BusyGuard { BUSY_COUNT.fetch_add(1, Ordering::SeqCst); BusyGuard }
pub fn is_busy() -> bool { BUSY_COUNT.load(Ordering::SeqCst) > 0 }

pub struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) { BUSY_COUNT.fetch_sub(1, Ordering::SeqCst); }
}
```

挂接点（每个入口一行 `let _busy = busy_state::enter_busy();`）：
1. `controllers/analyze.rs` 全盘扫描入口（spawn_blocking 任务闭包内）
2. `controllers/clean.rs` apply 阶段（execute 模式）
3. `controllers/uninstall.rs` 批量卸载任务内（已有 spawn_blocking，1290 行附近）
4. `controllers/optimize.rs` 优化执行入口（268 行 spawn_blocking 内）

**注意**：guard 必须 move 进实际执行的长任务闭包（对齐花笺 keep-awake 一节 5.2 的注意事项），不能在 command 函数体里拿了就 drop。

### 3.2 主体：Dock 退出拦截（新文件，~70 行）

```rust
// src-tauri/src/macos_dock_quit.rs
use std::sync::{Once, OnceLock};
use objc2::{ffi, runtime::{AnyClass, AnyObject, Imp, Sel}, sel};
use objc2_app_kit::NSApplicationTerminateReply;
use tauri::{AppHandle, Emitter, Manager};

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static INSTALL: Once = Once::new();
/// 前端确认后置 true，下一次 applicationShouldTerminate 放行。
static CONFIRMED_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn install_dock_quit_handler(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());
    INSTALL.call_once(|| {
        let Some(delegate_class) = AnyClass::get(c"TaoAppDelegateParent") else {
            log::warn!("[dock-quit] TaoAppDelegateParent not found; Dock 退出将不做拦截");
            return;  // 降级为现有行为：裸退出
        };
        let imp: Imp = unsafe {
            std::mem::transmute(
                application_should_terminate
                    as extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> NSApplicationTerminateReply,
            )
        };
        let added = unsafe {
            ffi::class_addMethod(
                delegate_class as *const AnyClass as *mut AnyClass,
                sel!(applicationShouldTerminate:),
                imp,
                c"Q@:@".as_ptr(),
            )
        };
        if !added.as_bool() {
            log::warn!("[dock-quit] applicationShouldTerminate: 已存在，未注入");
        }
    });
}

extern "C-unwind" fn application_should_terminate(
    _delegate: &AnyObject, _sel: Sel, _sender: &AnyObject,
) -> NSApplicationTerminateReply {
    use std::sync::atomic::Ordering;
    // 前端已确认过 → 放行
    if CONFIRMED_EXIT.swap(false, Ordering::SeqCst) {
        return NSApplicationTerminateReply::TerminateNow;
    }
    // 无长任务 → 直接放行（安静退出，不弹框打扰）
    if !crate::lib::core::busy_state::is_busy() {
        return NSApplicationTerminateReply::TerminateNow;
    }
    // 有任务在跑 → 取消本次终止，交前端确认
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("dock-quit-requested", ());
    }
    NSApplicationTerminateReply::TerminateCancel
}

/// 前端确认对话框点击"退出"后调用的 command。
#[tauri::command]
pub fn confirm_dock_quit() {
    CONFIRMED_EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(app) = APP_HANDLE.get() {
        // 重新发起终止 → 会再次进入 applicationShouldTerminate: → 放行
        app.cleanup_before_exit();
        app.exit(0);
    }
}
```

### 3.3 前端侧（~40 行）

1. `src/constants/tauri-events.ts` 登记 `dock-quit-requested`；
2. `src/constants/tauri-commands.ts` 登记 `confirm_dock_quit`；
3. App 层（`src/App.tsx` 或 Shell 层）监听事件 → 弹确认框（复用现有 React 确认组件）：

```tsx
// 文案要点：明示哪个任务在跑、退出后果
// 「磁盘分析正在进行中，退出将中断任务。确定退出 MoleStudio？」
listen('dock-quit-requested', async () => {
  const ok = await showQuitConfirmDialog()   // 现有 React 组件
  if (ok) await invoke('confirm_dock_quit')
  // 取消则什么都不做：terminate 已被 TerminateCancel，应用继续运行
})
```

### 3.4 挂接点（lib.rs setup 内一行）

`src-tauri/src/lib.rs` setup 闭包内（主窗口事件注册之前）：

```rust
#[cfg(target_os = "macos")]
crate::macos_dock_quit::install_dock_quit_handler(handle);
```

新文件在 `main.rs`/`lib.rs` 顶部 `#[cfg(target_os = "macos")] mod macos_dock_quit;`，command 注册进 `invoke_handler`。

## 四、验证步骤（macOS 上机，15 分钟）

```bash
# ① 空闲退出：无任务时 Dock 右键 → 退出 → 预期直接退出，不弹框
# ② 扫描中退出：发起全盘扫描 → Dock 退出 → 预期弹确认框；
#    点取消 → 应用继续、扫描继续（进度条不动即失败）
# ③ 确认退出：同上弹框 → 点退出 → 预期应用正常退出、进程消失（ps 查无 mole 进程）
# ④ panic 安全：扫描中途用 lldb detach 模拟任务 panic → BusyGuard Drop 后计数归零，
#    再 Dock 退出应直接放行（验证 RAII 不漏计数）
# ⑤ dev 模式验证类名：注入失败时 log 有 "[dock-quit] TaoAppDelegateParent not found"，
#    此时需上机确认我们 Tauri 版本的 delegate 类名（见风险①）
```

## 五、风险速查

| 风险 | 说明 | 缓解 |
|------|------|------|
| delegate 类名不匹配 | `TaoAppDelegateParent` 是 dbx 所用 tao 版本的类名，我们 Tauri 2.x 的 tao 版本可能不同 | 上机先跑验证⑤；若类名不同，从 tao 源码（`tao/src/platform_impl/macos/app_delegate.rs`）查实际类名 |
| BusyGuard 漏挂 | 某长任务入口忘加 guard → 该任务期间退出不弹框，退化为现状 | 代码评审 checklist；四个挂接点在本文 3.1 列死 |
| app.exit(0) 不触发 applicationShouldTerminate | 若 Tauri exit 走 `exit()` 直接杀，确认框会卡死 | 实测③为准；备选改用 `[NSApp terminate:nil]`（经 objc2 msg_send 主动发起 terminate，必走回调） |
| 前端未就绪时 Dock 退出 | webview 未加载完、事件没人听 → 弹框永远不来，应用退不掉 | `applicationShouldTerminate:` 里加 `frontend_ready` 检查（借鉴 dbx CloseBehaviorState）：未就绪直接 TerminateNow |
| 与"隐藏到托盘"语义冲突 | 用户可能预期 Dock 退出 = 关窗口（隐藏），而非退出进程 | 保持现状语义：Dock 退出就是真退出（唯一出口）；确认框文案写明 |