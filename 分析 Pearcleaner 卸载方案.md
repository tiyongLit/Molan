# 分析 Pearcleaner 卸载方案

> **Workspace:** /home/a16/work/molestudio

---

**User**

`/home/a16/work/molestudio/src-tauri/src/controllers/uninstall.rs#L1-1302` /home/a16/work/molestudio/src/pages/Uninstall 这个你帮我分析我们的这个tabs 卸载我们对齐的是mole /home/a16/macos/mvp/Mole/bin/uninstall.sh 跟 /home/a16/macos/mvp/Burrow 它两个参考复刻的, 现在我需要分析下 /home/a16/macos/mvp/Pearcleaner 它怎么样,我需要你最好的参考方式 你分析下我希望卸载最干净最没有残留的方案,我们还是不自己发挥,核心思想我们还是借鉴别的,你帮我分析.

**Assistant**

分析这个卸载功能对齐方案需要先并行读取当前实现和各个参考项目。我先做一次内存快速检查，然后并行读取关键文件。

我先继续并行读取关键实现细节：当前 Rust 后端的卸载执行逻辑、前端 UninstallTab、Mole 的 batch.sh（实际清理逻辑）、Burrow 卸载、以及 Pearcleaner 的结构。

让我探索 Pearcleaner 目录结构，以及 Burrow 的卸载计划实现，这是关键参考。

Pearcleaner 找到了。让我并行读取 Pearcleaner 的关键卸载逻辑文件，以及当前项目 UninstallTab UI。

让我找到 Pearcleaner 真实文件路径并读取关键逻辑。

让我确认 Pearcleaner 的实际文件路径结构。

路径是 `/home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/`。并行读取 Pearcleaner 关键逻辑文件。

让我读取 Pearcleaner 的位置定义和 Mole 的 find_app_files 逻辑，以及当前项目的 batch 卸载实现。

让我读取 Mole 的 find_app_files 实现（残留路径扫描核心）和 Pearcleaner 的 PKG receipt 逻辑。

我已经掌握了足够信息。下面是基于当前实现 + Mole + Burrow + Pearcleaner 的对照分析，以及"最干净无残留"的借鉴方案建议。

---

# 一、现状盘点：你们已经复刻了什么

## 1.1 Rust 后端 [uninstall.rs](file:///home/a16/work/molestudio/src-tauri/src/controllers/uninstall.rs)

**已对齐 Mole 的部分（来自 `bin/uninstall.sh` + `lib/uninstall/batch.sh`）**：

- **应用枚举** [mole_list_apps](file:///home/a16/work/molestudio/src-tauri/src/controllers/uninstall.rs#L72)：扫 `/Applications`、`~/Applications`、`/Library/Input Methods`、`~/Library/Input Methods`、`/Volumes/*/Applications`、pkg_receipts 非标准路径。mdls+du 双轨估容，7 天 mtime 缓存。
- **预览/执行分离**：dry_run 走 [collect_app_details](file:///home/a16/work/molestudio/src-tauri/src/controllers/uninstall.rs#L571)，execute 走 `batch::batch_uninstall_applications`。
- **残留路径扫描**：通过 `crate::uninstall::batch` 调 Mole 的 `find_app_files`/`find_app_system_files` 等价逻辑，按 bundle_id + app_name 多变体匹配一组固定 well-known 路径。
- **Sibling guard**（[Mole 同名逻辑](file:///home/a16/macos/mvp/Mole/lib/uninstall/batch.sh#L936-L980)）：同 bundle_id 还在的兄弟安装（Xcode vs Xcode-beta）→ 不动 bundle_id 派生的残留。

## 1.2 前端 [Uninstall 页](file:///home/a16/work/molestudio/src/pages/Uninstall)

- [index.tsx](file:///home/a16/work/molestudio/src/pages/Uninstall/index.tsx)：三 Tab（卸载 / 更新 / 启动项），底部操作栏。
- [UninstallTab.tsx](file:///home/a16/work/molestudio/src/pages/Uninstall/UninstallTab.tsx#L40-L67)：行内展开残留审阅面板，已经把 Kind 分成 **Auto selected** vs **Needs review** 两组——这是 **Burrow [UninstallPreview.Kind.autoSelected](file:///home/a16/macos/mvp/Burrow/macos/Sources/UninstallPreview.swift#L25-L36) 的复刻**，做得对。

## 1.3 现状结论

| 维度 | 当前对齐 | 完成度 |
|---|---|---|
| 应用列表 + 大小 + 上次使用 | Mole | ✅ |
| 残留路径按 bundle_id 扫描 | Mole `find_app_files` | ✅ |
| Kind 分类 + Auto/Review 分组 | Burrow | ✅ |
| Sibling guard / 敏感数据 / Brew cask / LaunchServices | Mole | ✅（在 batch 模块里）|
| **Container UUID → bundle_id 解析** | ❌ | 缺 |
| **Group Container (App Group) 解析** | ❌ | 缺 |
| **Library 根 depth-2 厂商子目录扫描** | ❌ | 缺 |
| **Spotlight 补漏** | ❌ | 缺 |
| **Base bundle id 剥离（.helper/.agent/.daemon）** | ❌ | 缺 |
| **反向孤儿扫描** | ❌ | 缺 |
| **Clear Data 模式（保留 app，清数据）** | ❌ | 缺 |

---

# 二、Pearcleaner 强在哪：四个 Mole/Burrow 都没有的能力

源码：[AppPathsFetch.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift)、[ReversePathsFetch.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/ReversePathsFetch.swift)、[Locations.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/Locations.swift)、[PKGManager.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/PKG/PKGManager.swift)。

## 2.1 Container UUID → bundle_id 正确解析（最关键）

macOS 的 `~/Library/Containers/<UUID>/` 是 Apple 用 `containermanagerd` 管的，UUID 不含 bundle_id 字符串。**Mole 和当前实现靠"目录名 substring 匹配 bundle_id"——这在 UUID 情况下根本匹配不到**，导致容器残留被漏掉。

Pearcleaner 的正确做法（[AppPathsFetch.swift#L187-L213](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L187-L213)）：

```swift
let metadataPlistURL = directory.appendingPathComponent(".com.apple.containermanagerd.metadata.plist")
if let metadataDict = NSDictionary(contentsOf: metadataPlistURL),
   let applicationBundleID = metadataDict["MCMMetadataIdentifier"] as? String {
    if applicationBundleID == self.appInfo.bundleIdentifier {
        containers.append(directory)  // 确认这个 UUID 目录属于本 app
    }
}
```

Group Container 同理走 `FileManager.containerURL(forSecurityApplicationGroupIdentifier:)`，而不是字符串匹配。

## 2.2 Library 根 depth-2 扫描，捕厂商子目录

Pearcleaner 在 `~/Library`、`/Library` 这种根上把 `maxDepth` 调到 2（[AppPathsFetch.swift#L236-L243](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L236-L243)），并在 depth=2 命中时把**父目录**加入结果（如果不是标准 macOS 子目录）：

```
/Library/Objective-See/LuLu/...   →  加入 /Library/Objective-See/LuLu
~/Library/Microsoft/Edge/...      →  加入 ~/Library/Microsoft/Edge
```

Mole 的 `find_app_files` 是逐个 well-known 路径 1 层枚举，**漏掉厂商子目录**——这是 Pearcleaner 显著更干净的地方。

## 2.3 Base bundle id 剥离 + 三档灵敏度

Pearcleaner 在 [AppPathsFetch.swift#L114-L129](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L114-L129) 把 `com.objective-see.blockblock.helper` 剥成 `com.objective-see.blockblock`，用来匹配同源的 LaunchDaemon——这能抓到 Mole 漏的 helper/agent/daemon/xpc/updater 等。

三档灵敏度（[AppPathsFetch.swift#L404-L462](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L404-L462)）：

| 档位 | 匹配方式 | 适用 |
|---|---|---|
| **Strict**（默认）| 文件名**精确相等** bundle_id 或 app_name | 安全，与 Mole 现行为一致 |
| **Enhanced** | contains + 两段 bundle_id 后缀 + 剥版本号 | 多抓一些，少量误报 |
| **Deep** | + 厂商名（bundle id 第 2 段）+ Team ID + Entitlements + base bundle id | 抓开发工具残留 |

## 2.4 Spotlight 补漏 + 反向孤儿扫描

- **Spotlight 补漏**（[AppPathsFetch.swift#L466-L528](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L466-L528)）：用 `NSMetadataQuery` 查 `kMDItemDisplayName` / `kMDItemPath` / `kMDItemComment` / `kMDItemCopyright` / `kMDItemEncodingApplications`，5s 超时 + 500 条上限。抓 well-known 列表外的散落文件。
- **反向孤儿扫描**（[ReversePathsFetch.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/ReversePathsFetch.swift#L109-L142)）：扫一组 location，**过滤掉所有跟已装 app 相关**的文件，剩下的就是"已经卸载的 app 留下的孤儿"。这是"已经用其他方式卸载过的 app"的清道夫。

---

# 三、Pearcleaner 的红线 / 不能直接抄的部分

| 项 | 风险 | 替代方案 |
|---|---|---|
| **`PKReceipt.receiptsOnVolume`** ([PKGManager.swift#L16-L22](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/PKG/PKGManager.swift#L16-L22)) | 私有 PackageKit 框架，MAS 审核「私有 API」拒审风险 | 直接读 `/private/var/db/receipts/<bundle_id>.plist`（Mole 已经有 `pkg_receipts.rs`，扩展解析 BOM plist 即可，BOM 是公开格式）|
| **NSMetadataQuery** | 公开 API，但沙箱内权限受限 | MAS 版可降级为只扫 well-known 路径；full 版可启用 |
| **无 Sibling guard** | Xcode/Xcode-beta 会误删共享数据 | **不要抄**——保留 Mole 的 [uninstall_live_bundle_has_other_install](file:///home/a16/macos/mvp/Mole/lib/uninstall/batch.sh#L980-L1130) |
| **Deep 模式厂商名匹配** | 短名（"Alin"）误报 | Pearcleaner 自己加了 ≥5 字符阈值（[ReversePathsFetch.swift#L120-L124](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/ReversePathsFetch.swift#L120-L124)），照搬即可 |
| ** Pearcleaner 直接 `rm`** | 违反你们 [03_硬约束与红线.md 红线 4](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)（必须走 trash + 用户确认）| 保留 Mole `trash` crate 路径，不抄 Pearcleaner 的删除 |

---

# 四、推荐方案：四层叠加，借鉴不发明

> 核心：**Mole 当算法地基 + 安全地基，Pearcleaner 当"覆盖面"补丁，Burrow 当 UI 分类，再加一个 Pearcleaner 的孤儿扫描独立 Tab。**

## Layer 1：保留 Mole 现有底座（不动）

`sibling guard`、`敏感数据检测`、`brew cask --zap` 路由、`launchctl bootout`、`lsregister -gc`、`trash` crate、`sibling fingerprint` 二次校验——这些是 Mole 经过 issue #1222/#1339/#1340/#1383 反复修出来的硬逻辑，**别动**。

## Layer 2：把 Pearcleaner 的"覆盖面"补丁加进 `lib/uninstall/`

新建/扩展以下 Rust 模块（仍在 `src-tauri/src/lib/uninstall/`，对齐 [04_后端工作流.md 模块边界](file:///home/a16/work/molestudio/.trae/rules/04_后端工作流.md)）：

| 新增能力 | 落位 | 抄谁 |
|---|---|---|
| Container UUID → bundle_id 解析（读 `.com.apple.containermanagerd.metadata.plist` 的 `MCMMetadataIdentifier`） | `lib/core/container_resolver.rs` | Pearcleaner [AppPathsFetch.swift#L187-L213](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L187-L213) |
| Group Container 解析（`containerURL(forSecurityApplicationGroupIdentifier:)`，Rust 用 `objc2` 调 NSFileManager） | 同上 | 同上 |
| Library 根 depth-2 + 厂商子目录父级收录 | 扩展 `find_app_files` 等价 Rust 实现 | Pearcleaner [AppPathsFetch.swift#L236-L282](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L236-L282) |
| Base bundle id 剥离（`.helper`/`.agent`/`.daemon`/`.service`/`.xpc`/`.launcher`/`.updater`） | 加进 `find_app_files` 的 pattern 集 | Pearcleaner [AppPathsFetch.swift#L114-L129](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L114-L129) |
| 三档灵敏度（Strict/Enhanced/Deep） | `embedded_rules.rs` 编译期常量 + 前端设置项 | Pearcleaner [AppPathsFetch.swift#L404-L462](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L404-L462) |
| Spotlight 补漏（full 版启用，mas 版跳过） | `lib/uninstall/spotlight_supplement.rs`，用 `objc2` 调 NSMetadataQuery | Pearcleaner [AppPathsFetch.swift#L466-L528](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L466-L528) |
| PKG BOM 文件清单（公开 plist 解析，不走私有框架） | 扩展现有 `lib/core/pkg_receipts.rs` | Mole 已有底座 + Pearcleaner [PKGManager.swift#L62-L86](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/PKG/PKGManager.swift#L62-L86) 的思路（但用 plist 解析代替 `PKReceipt` 私有 API） |

## Layer 3：Burrow 的 UI 补丁（部分已做，补两个）

你们 [UninstallTab.tsx 的 Kind 分类](file:///home/a16/work/molestudio/src/pages/Uninstall/UninstallTab.tsx#L19-L39) 已经抄了 Burrow。再补两个 Burrow 的好东西：

- **"Clear Data" 模式**（[Burrow UninstallPlan.dataOnly](file:///home/a16/macos/mvp/Burrow/macos/Sources/UninstallPlan.swift#L13-L16)）：保留 `.app`，只清残留。一行 filter，加个底部按钮"清数据不卸载"，对应"重置应用"语义。
- **Input method 检测**（[Burrow UninstallPlan.isInputMethod](file:///home/a16/macos/mvp/Burrow/macos/Sources/UninstallPlan.swift#L18-L21)）：`/Library/Input Methods/` 或 `.inputmethod` 后缀的 app 单独标记，给用户额外提示。

## Layer 4：新增"孤儿残留"Tab（Pearcleaner 反向扫描，新功能）

这是**真正实现"最干净"的关键**——用户在用 MoleStudio2 之前用其他方式（拖到废纸篓）卸过的 app 留下的残留，靠 Layer 1-3 抓不到（因为根本没有目标 app 可作为 `find_app_files` 的输入）。

 Pearcleaner [ReversePathsFetch](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/ReversePathsFetch.swift#L109-L142) 的逻辑就是反向：扫 [Locations.reverse.paths](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/Locations.swift#L150-L172) 一组固定路径，**逐项检查是否与任一已安装 app 相关**，不相关的就是孤儿。

落地建议：

- 前端：`src/pages/Uninstall/OrphansTab.tsx`，作为第四个 Tab「残留孤儿」。
- 后端：`controllers/uninstall.rs` 加 `mole_orphan_scan()` Tauri command，复用 Layer 1 的 `mole_list_apps` 拿到已装 app 列表 → 反向扫 `Locations.reverse.paths` → 过滤 → 返回孤儿列表。
- 删除走同一套 `trash` + 用户确认 + sibling guard（防误删共享 group container）。

---

# 五、优先级与落地顺序建议

| 阶段 | 做什么 | 收益 | 难度 |
|---|---|---|---|
| **P0** | Container UUID 解析（Layer 2 第 1 项） | 修最大遗漏盲区——所有 sandbox app 的容器残留 | 中（plist 解析）|
| **P0** | Group Container 解析 | 抓 App Group 共享数据（微信、Telegram 等）| 中（要桥 NSFileManager）|
| **P1** | Library depth-2 + 厂商子目录 + base bundle id 剥离 | 抓厂商目录下的 helper/agent/daemon | 低（纯路径逻辑）|
| **P1** | "Clear Data" 模式 + Input method 标记 | UX 补齐 Burrow | 低（前端为主）|
| **P2** | Spotlight 补漏（仅 full 版） | 抓非标准位置散落文件 | 高（objc2 桥 + 沙箱限制）|
| **P2** | PKG BOM plist 解析扩展 | pkg 安装的 app 精确清 | 中 |
| **P3** | 孤儿扫描 Tab | 清"史前残留"，真正"最干净" | 中（新 Tab + 反向匹配）|
| **P3** | 三档灵敏度设置 | 让用户在"漏报"和"误报"间自调 | 低（已有逻辑分级）|

---

# 六、一句话总结

- **Mole** 给"安全地基 + bundle_id 算法骨架"——别动；
- **Burrow** 给"UI Kind 分类 + Clear Data + Input Method"——已经抄了大半，补齐即可；
- **Pearcleaner** 给"覆盖面四件套"：Container UUID 解析、Group Container、Library depth-2 + base bundle id、Spotlight 补漏——**这是你们当前最大的盲区，也是"最干净"的关键**；
- **Pearcleaner 反向孤儿扫描**——独立 Tab，是"已经用其他工具卸过的残留"的清道夫，**这一项让 Pearcleaner 在"无残留"维度上显著超过 Mole 和 Burrow**。

需要我下一步把 P0 项（Container UUID 解析 + Group Container）的 Rust 实现改动预览出来吗？落位会是 `src-tauri/src/lib/core/container_resolver.rs`（新文件）+ 在 `lib/uninstall/` 的 `find_app_files` 等价实现里调用它。

---

**User**

那你看看/home/a16/macos/lemon-cleaner它的卸载了,你推荐一个卸载最干净的方案我们tauri里面用

**Assistant**

我来探索 lemon-cleaner 的卸载实现。根据规则文件，lemon-cleaner 是"UI/UX 交互基准"，但用户明确要看它的卸载实现细节。

Lemon 主体在 `Lemon/` 下，但卸载的核心逻辑应该在 `LemonDaemon` 里。我先看 controller 和 ToolConfig（交互范式），同时找卸载核心。

奇怪，按理 LS 应该看到。让我直接 grep 全仓 uninstall 关键词，并用更宽松的路径搜 LemonDaemon。

Lemon 真正的卸载实现在 `localPod/LemonUninstaller/` 和 `localPod/QMCoreFunction/Classes/McUninstaller/`。让我读核心：扫描器、uninstall.xml、LMSearchPath、McUninstallSoft。

核心扫描逻辑在 `McSoftwareFileScanner`。这是 Lemon 卸载的真正算法核心。

太完整了。我快速看 Lemon 的 `McLocalSoft`（决定 bundleID/appName/executableName/companyName 怎么算出来）和 `PkgUninstallManager`（pkg 路线），就可以给出整合方案。

我已经掌握 Lemon 卸载的全部核心算法。下面给出整合方案。

---

# 一、Lemon 卸载实现：核心算法一览

Lemon 把卸载逻辑拆成三个层次：

| 层 | 文件 | 职责 |
|---|---|---|
| **算法核心** | [McSoftwareFileScanner.m](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m) | 按"分类 → 路径 → 选项"二维矩阵扫描，输出"分类 → 路径列表"的字典 |
| **路径常量** | [LMSearchPath.m](file:///home/a16/macos/lemon-cleaner/localPod/LemonUninstaller/LemonUninstaller/Classes/Manager/LMSearchPath.m) | 7 类 well-known 路径 |
| **App 扫描** | [McApplicationScanner.m](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McLocalSoft/McScanner/McApplicationScanner.m#L11-L35) | 用 `_LSCopyAllApplicationURLs` 私有 API 列出所有已装 app |
| **pkg 路线** | [PkgUninstallManager.m](file:///home/a16/macos/lemon-cleaner/localPod/LemonUninstaller/LemonUninstaller/Classes/Manager/Pkg/PkgUninstallManager.m) | `pkgutil --pkgs` + 白名单到 pkg bundle id 的硬编码映射（NTFS/BeeCut/Cisco） |

## 1.1 Lemon 的"扫描矩阵"——这是它的核心价值

[McSoftwareFileScanner.m#L196-L302](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L196-L302)：

| 分类 | 路径集 | 选项（匹配方式）| 后缀正则 |
|---|---|---|---|
| **Support（支持文件）**| `~/Library/Application Support`、`/Library/Application Support` | name \| bundleId \| **company** | 无 |
| **Cache（缓存）**| `~/Library/Caches`、`/Library/Caches`、`$TMPDIR`、`$TMPDIR/../C` | name \| bundleId \| company | 无 |
| **Preferences（设置）**| `~/Library/Preferences`、`/Library/Preferences` | **bundleId only** | 无 |
| **State（保存状态）**| `~/Library/Saved Application State` | bundleId only | 无 |
| **CrashReporter（崩溃日志）**| `~/Library/Application Support/CrashReporter`、`~/Library/Logs/DiagnosticReports` 等 | name \| bundleId \| company | `_-(([0-9a-fA-F-]{5,})|([0-9]{4}(-[0-9]{1,2}){2})).*`（剥 UUID/日期后缀）|
| **Logs（日志）**| `~/Library/Logs`、`/Library/Logs` | name \| bundleId \| company | 无 |
| **Sandbox（沙盒）**| `~/Library/Containers` | bundleId only + **Container.plist 二次确认** | 无 |
| **Daemon（启动项）**| `~/Library/LaunchAgents`、`/Library/LaunchAgents`、`/Library/LaunchDaemons`、`/Library/StartupItems` | bundleId only，**plist 内容二次确认** | `.plist` |

## 1.2 Lemon 三种匹配维度

[LMSearchPath.m 里的 `kMcSearchByName|kMcSearchByBundleID|kMcSearchByCompany`](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L437-L462)：

- **ByName**：用 `appName`（去空格、小写）做正则词边界匹配
- **ByBundleID**：用 `bundleID`（去空格、小写）做正则词边界匹配
- **ByCompany**：bundleID 是三段时（`com.<company>.<product>`），取第二段当公司名，匹配"公司名目录"下的"产品名/bundleId"子项。这是 Pearcleaner Deep 模式的简化版，但 Lemon 把它写死在 Support/Cache/CrashReporter/Logs 四类上。

## 1.3 Lemon 三个独有的关键技巧

1. **Container.plist 二次确认**（[McSoftwareFileScanner.m#L307-L336](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L307-L336)）：扫 `~/Library/Containers` 时，除了 bundleId 匹配，**还要打开每个 `Container.plist`**，读 `SandboxProfileDataValidationInfo.SandboxProfileDataValidationParametersKey.application_bundle / application_dyld_paths`，看它是否以 `soft.bundlePath` 开头。这是 Lemon 抓 UUID 容器的方式——但比 Pearcleaner 的 `MCMMetadataIdentifier` 老， Pearcleaner 用更新更稳的 `containermanagerd.metadata.plist`。

2. **LaunchDaemon plist 内容二次确认**（[McSoftwareFileScanner.m#L338-L393](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L338-L393)）：bundleId 模糊匹配 `.plist` 后，**打开 plist 读 Program / ProgramArguments**，检查可执行路径是否命中之前已扫到的路径集合。这避免"daemon 名字碰巧撞上"的误报。

3. **路径父子去重**（[McSoftwareFileScanner.m#L140-L180 + filepathExistsArray#L578-L607](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L140-L180)）：所有分类扫完后倒序遍历，**子路径被父路径包含则删子，反之保留父删子**。例如 `~/Library/Application Support/Chrome` 被扫到后，`~/Library/Application Support/Chrome/Default` 不会重复出现。这是 Pearcleaner 也做的事（[AppPathsFetch.swift#L505-L518](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift#L505-L518)），但 Lemon 的实现更直观、更易移植到 Rust。

## 1.4 Lemon 的红线（不能抄的部分）

| Lemon 做法 | 问题 | 替代 |
|---|---|---|
| `_LSCopyAllApplicationURLs` 私有 API 列已装 app | MAS 拒审 | 你们已经用 `find /Applications -name *.app -maxdepth 3` + pkg receipts，**继续保留** |
| `pkgutil --pkgs` shell 调用 | 违反 [03 红线 1](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)（禁外部 shell）| 你们已经有 Rust `pkg_receipts.rs`，扩展即可 |
| `get_pkg_path.sh` | shell 脚本 | 同上 |
| `uninstall.xml` 硬编码特殊 app 残留 | 外置规则，违反 [03 红线 3](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md) | 你们已有 `embedded_rules.rs`，把特殊 app 残留迁移过去 |
| Lemon 直接 `rm` | 违反 [03 红线 4](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md) | 保留你们 Mole 来的 `trash` crate |

---

# 二、四个项目对照：覆盖面维度

把"卸载能扫到的东西"拆成 11 项，逐项对照：

| 维度 | Mole `find_app_files` | Burrow | Pearcleaner | **Lemon** | 你们当前 |
|---|---|---|---|---|---|
| Application Support（按 bundleId / name）| ✅ | ✅ | ✅ | ✅ + **company 目录剥到产品级** | ✅ |
| Caches | ✅ | ✅ | ✅ | ✅ + **`$TMPDIR`、`$TMPDIR/../C`** | ⚠️ 缺 `$TMPDIR` |
| Preferences（bundleId only）| ✅ | ✅ | ✅ | ✅ | ✅ |
| Saved Application State | ✅ | ✅ | ✅ | ✅ | ✅ |
| CrashReporter（剥 UUID/日期后缀）| ⚠️ 部分 | ⚠️ 部分 | ⚠️ 部分 | ✅ **正则剥后缀** | ⚠️ 缺正则 |
| Logs（含 DiagnosticReports）| ✅ | ✅ | ✅ | ✅ | ✅ |
| Containers（UUID 解析）| ❌ | ❌ | ✅ **MCMMetadataIdentifier** | ⚠️ **老式 Container.plist** | ❌ |
| Group Containers（App Group）| ❌ | ❌ | ✅ | ❌ | ❌ |
| LaunchAgents/Daemons（plist 内容二次确认）| ⚠️ 只 bundleId | ❌ | ✅ base bundle id 剥离 | ✅ **Program 路径交叉验证** | ⚠️ |
| **Library depth-2 厂商目录**（`/Library/Objective-See/LuLu`）| ❌ | ❌ | ✅ | ⚠️ company 模式接近 | ❌ |
| **路径父子去重**（避免重复列出父+子）| ⚠️ | ❌ | ✅ | ✅ **倒序算法最清晰** | ⚠️ |

**结论**：Lemon 的优势是**"扫描矩阵 + 路径去重 + plist 内容二次确认"**这三大件算法骨架，劣势是它的 Container 解析方法已过时（Pearcleaner 的 `MCMMetadataIdentifier` 才对），且大量用私有 API/shell。

---

# 三、推荐方案：**Mole 安全地基 + Lemon 算法骨架 + Pearcleaner 容器解析补丁 + Burrow UI 分类**

## 3.1 核心思想

**不重新发明轮子**。把 Lemon 的 [McSoftwareFileScanner](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m) 当算法骨架翻译成 Rust（这是 Lemon 经过腾讯多年沉淀、最稳定的部分），再用 Pearcleaner 的 `MCMMetadataIdentifier` 修掉 Lemon 容器解析的老 bug，最后保留 Mole 的 sibling guard / trash / brew zap 当安全地基。

## 3.2 落位

按 [04_后端工作流.md 模块边界](file:///home/a16/work/molestudio/.trae/rules/04_后端工作流.md)，全部进 `src-tauri/src/lib/uninstall/`：

```
src-tauri/src/lib/uninstall/
├── scanner.rs          ← 新：扫描矩阵（Lemon 算法骨架的 Rust 翻译）
├── container_resolver.rs ← 新：Container UUID + Group Container（Pearcleaner 方案）
├── daemon_verifier.rs  ← 新：LaunchDaemon plist 内容二次确认（Lemon 算法）
├── path_dedup.rs       ← 新：路径父子去重（Lemon 倒序算法）
├── batch.rs            ← 现有，保留 Mole 的 sibling guard / trash 路由
└── brew.rs             ← 现有
```

## 3.3 扫描矩阵的 Rust 设计（推荐方案核心）

把 Lemon 的"分类 × 路径 × 选项 × 后缀正则"四元组固化成编译期静态表（对齐 [03 红线 3](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)——规则编译期内嵌）：

```rust
// src-tauri/src/lib/uninstall/scanner.rs

use MatchOption::*;
use FileKind::*;

#[derive(Clone, Copy, PartialEq)]
pub enum MatchOption { ByName, ByBundleId, ByCompany }

#[derive(Clone, Copy, PartialEq)]
pub enum FileKind {
    Bundle, Support, Cache, Preferences, State,
    CrashReporter, Logs, Sandbox, Daemon, Other,
}

/// 一行扫描矩阵。所有规则编译期内嵌（embedded_rules.rs），不读外置 xml。
pub struct ScanRule {
    pub kind: FileKind,
    pub paths: &'static [&'static str],       // well-known 路径
    pub options: &'static [MatchOption],       // ByName/ByBundleId/ByCompany 组合
    pub suffix_regex: Option<&'static str>,    // 后缀正则（如 CrashReporter 的 UUID/日期）
    pub auto_selected: bool,                   // Burrow classify: 安全项 vs 审阅项
}

pub const SCAN_RULES: &[ScanRule] = &[
    // Application Support：name | bundleId | company
    ScanRule { kind: Support, paths: &["/Library/Application Support", "~/Library/Application Support"],
               options: &[ByName, ByBundleId, ByCompany], suffix_regex: None, auto_selected: true },
    // Caches：补 Lemon 的 $TMPDIR 和 $TMPDIR/../C
    ScanRule { kind: Cache, paths: &["/Library/Caches", "~/Library/Caches", "$TMPDIR", "$TMPDIR/../C"],
               options: &[ByName, ByBundleId, ByCompany], suffix_regex: None, auto_selected: false },
    // Preferences：仅 bundleId（Lemon 选择，防误删）
    ScanRule { kind: Preferences, paths: &["/Library/Preferences", "~/Library/Preferences"],
               options: &[ByBundleId], suffix_regex: None, auto_selected: true },
    // Saved State：仅 bundleId
    ScanRule { kind: State, paths: &["~/Library/Saved Application State"],
               options: &[ByBundleId], suffix_regex: None, auto_selected: true },
    // CrashReporter：剥 UUID/日期后缀（Lemon 的关键贡献）
    ScanRule { kind: CrashReporter,
               paths: &["/Library/Application Support/CrashReporter", "/Library/Logs/DiagnosticReports",
                         "~/Library/Application Support/CrashReporter", "~/Library/Logs/DiagnosticReports"],
               options: &[ByName, ByBundleId, ByCompany],
               suffix_regex: Some(r"_(([0-9a-fA-F-]{5,})|([0-9]{4}(-[0-9]{1,2}){2})).*"),
               auto_selected: false },
    // Logs
    ScanRule { kind: Logs, paths: &["/Library/Logs", "~/Library/Logs"],
               options: &[ByName, ByBundleId, ByCompany], suffix_regex: None, auto_selected: false },
    // Containers：不靠名字，走 container_resolver.rs（Pearcleaner 方案）
    ScanRule { kind: Sandbox, paths: &["~/Library/Containers"],
               options: &[], suffix_regex: None, auto_selected: true },
    // Daemon：bundleId 匹配后做 plist 内容二次确认（daemon_verifier.rs）
    ScanRule { kind: Daemon,
               paths: &["/Library/LaunchAgents", "/Library/LaunchDaemons", "/Library/StartupItems",
                         "~/Library/LaunchAgents", "~/Library/LaunchDaemons"],
               options: &[ByBundleId], suffix_regex: Some(r"\.plist$"), auto_selected: true },
];
```

## 3.4 三个补丁模块（对应 Lemon 算法被 Pearcleaner/现代实践补的地方）

**1. `container_resolver.rs`（Pearcleaner 修 Lemon 的老 bug）**

Lemon 用 `Container.plist` 的 `application_bundle` 字段，但新 macOS 已经改成 `containermanagerd.metadata.plist` 里的 `MCMMetadataIdentifier`（Pearcleaner 已验证）。你们要直接抄 Pearcleaner 的正确做法：

```rust
// 读 ~/Library/Containers/<UUID>/.com.apple.containermanagerd.metadata.plist
// 取 MCMMetadataIdentifier 字段，== app.bundle_id 即属于本 app
```

Group Container 走 `containerURL(forSecurityApplicationGroupIdentifier:)`，Rust 用 `objc2` 桥 NSFileManager。

**2. `daemon_verifier.rs`（Lemon 算法骨架，验证 plist 内容）**

[Lemon 的 LaunchDaemon 二次确认](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L338-L393)：bundleId 模糊匹配 plist 文件名后，**读 plist 的 Program / ProgramArguments**，看可执行路径是否命中已扫到的路径集合。这避免"daemon 名字碰巧撞上"的误报，**Rust 直接翻译**。

**3. `path_dedup.rs`（Lemon 倒序去重算法）**

[Lemon 的 `filepathExistsArray`](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L578-L607) 的 Rust 翻译：所有分类扫完后，倒序遍历每个分类，子路径被父路径包含则删子，反之保留父删子。这是输出"干净的扁平路径列表"的关键。

## 3.5 Sibling guard：保留 Mole，**不要抄 Lemon**

Lemon 没有 sibling guard——它会误删 Xcode 和 Xcode-beta 共享的 `~/Library/Preferences/com.apple.dt.Xcode.plist`。Mole 的 [uninstall_live_bundle_has_other_install](file:///home/a16/macos/mvp/Mole/lib/uninstall/batch.sh#L980-L1130) 是这部分最稳的，**保留你们的 Rust 移植**，在扫描矩阵输出后、执行删除前，对每条 bundle_id 派生路径过一遍 sibling guard。

## 3.6 与 Pearcleaner 方案的取舍

我上一轮给的"Pearcleaner 三档灵敏度 + Spotlight 补漏 + 反向孤儿扫描"方案，与这一轮的"Lemon 矩阵 + Lemon 去重 + Pearcleaner 容器解析"方案，**不冲突，是叠加关系**：

| 能力 | 来源 | 优先级 | 理由 |
|---|---|---|---|
| **扫描矩阵 + 后缀正则 + plist 内容验证 + 路径去重** | Lemon | **P0** | 这四件是任何卸载工具的算法骨架，Lemon 实现最清晰，Rust 翻译工作量最小 |
| **Container UUID 解析（MCMMetadataIdentifier）** | Pearcleaner | **P0** | Lemon 老方法在新 macOS 上抓不全 |
| **Group Container 解析** | Pearcleaner | **P0** | Mole/Burrow/Lemon 都没有 |
| **Library depth-2 + base bundle id 剥离** | Pearcleaner | **P1** | 补 Lemon 矩阵的盲区 |
| **三档灵敏度** | Pearcleaner | **P2** | 让用户调"漏报/误报"权衡 |
| **Spotlight 补漏** | Pearcleaner | **P3** | 仅 full 版，补 well-known 之外的散落 |
| **反向孤儿扫描** | Pearcleaner | **P3** | 独立 Tab，清"史前残留" |

---

# 四、最终一句话方案

> **"Mole 当安全地基（sibling guard + trash + brew zap），Lemon 当算法骨架（扫描矩阵 + 后缀正则 + plist 内容验证 + 路径去重），Pearcleaner 当容器解析补丁（修 Lemon 容器老方法 + 加 Group Container），Burrow 当 UI 分类"——这是当前你们在 Tauri/Rust 栈下能做到的最干净卸载方案。**

落地顺序：

1. **P0**：在 `lib/uninstall/` 新建 `scanner.rs`（翻译 Lemon 矩阵）+ `container_resolver.rs`（Pearcleaner 容器解析）+ `daemon_verifier.rs`（Lemon plist 验证）+ `path_dedup.rs`（Lemon 去重），把现有 `batch.rs` 的 `find_app_files` 等价实现替换成新矩阵驱动。
2. **P1**：在 `embedded_rules.rs` 加特殊 app 残留（替换 Lemon 的 `uninstall.xml` 硬编码）+ Library depth-2 + base bundle id 剥离。
3. **P3**：孤儿扫描 Tab + Spotlight 补漏。

要我把 P0 的 `scanner.rs` 骨架（Lemon 矩阵 + 三种匹配维度 + 路径去重的 Rust 完整翻译）以 `show_diff` 形式预览出来吗？这是新文件，按规则我会用代码块直接给出。

---

**User**

你在帮我分析下这个/home/a16/macos/mvp/PureMac 它有我们可以借鉴的? 它好像也是卸载的, 它能卸载干净2?

**Assistant**

我先探索 PureMac 的结构。

PureMac 结构非常清晰。从命名上看 `AppPathFinder.swift`、`Conditions.swift`、`Locations.swift`、`OrphanSafetyPolicy.swift`、`CleaningEngine.swift`、`Orphans/OrphanListView.swift`——它兼具 Pearcleaner 的路径扫描 + 孤儿扫描。我并行读取核心扫描与安全策略文件。

PureMac 实际上是 **Pearcleaner 的 fork/重写版**——`AppPathFinder` 和 Pearcleaner 的几乎一样（三档灵敏度、Container UUID、base bundle id、Library depth-2、filterSubpaths 全有）。但 PureMac 有 **两个 Pearcleaner 没有的关键增强**。我读 OrphanListView、Conditions、CleaningEngine、Models 来确认它的独有价值。

PureMac 在 Pearcleaner 基础上做了 **三个关键的工程化改进**，这三个是 Pearcleaner 没有的，对你们 Tauri/Rust 实现非常有借鉴价值。再看 OrphanListView 和 CleaningEngine 的执行路径，确认它的删除安全。

所有关键信息已经掌握。下面是基于 PureMac 的对比分析与方案更新。

---

# 一、PureMac 是什么

**PureMac 是 Pearcleaner 的工程化重写**，技术栈换成了纯 SwiftUI + actor + async/await，但底层算法骨架（三档灵敏度、Container UUID 解析、base bundle id、Library depth-2、filterSubpaths）和 Pearcleaner 完全一致。从 [AppPathFinder.swift](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/AppPathFinder.swift) 和 Pearcleaner [AppPathsFetch.swift](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/AppPathsFetch.swift) 对照可以看出，**PureMac = Pearcleaner 算法 + 现代工程实践 + 安全加固**。

它能不能卸载干净？**能**。它的覆盖面和 Pearcleaner 完全相同（同样有 Container UUID + Group Container + Spotlight 补漏 + 反向孤儿扫描），所以从"干净度"上和 Pearcleaner 同级，**比 Mole/Burrow/你们当前实现更干净**（特别是在 sandbox app 容器和 group container 维度）。

---

# 二、PureMac 比 Pearcleaner 多出来的三个关键增强

这是 PureMac 真正有借鉴价值的地方。这三个是 Pearcleaner 没有或更弱的：

## 2.1 高风险 dotfile 黑名单（防 CLI 工具被误删）

[Conditions.swift#L362-L402](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/Conditions.swift#L362-L402) 定义了一个 `highRiskHomeDotPaths` 列表：

```swift
let highRiskHomeDotPaths: [String] = [
    "\(home)/.claude", "\(home)/.ssh", "\(home)/.aws", "\(home)/.gnupg",
    "\(home)/.kube", "\(home)/.docker", "\(home)/.config", "\(home)/.git",
    "\(home)/.gitconfig", "\(home)/.netrc", "\(home)/.npmrc", "\(home)/.cargo",
    "\(home)/.rustup", "\(home)/.password-store", "\(home)/.vscode", "\(home)/.vim",
    "\(home)/.zshrc", "\(home)/.bashrc", "\(home)/.bash_profile", "\(home)/.profile", ...
]
```

[Locations.swift#L30-L35](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/Locations.swift#L30-L35) 的注释专门解释了为什么**不扫 bare `$HOME`**：

> User home - bare "\(home)" is intentionally NOT scanned.
> Scanning bare $HOME matches top-level dotfiles like .claude,
> .ssh, .aws, .kube by normalized app-name ("claude" matching
> ".claude") and invites data loss when uninstalling unrelated
> webapps. Scoped subdirs below are still scanned.

**这是 Pearcleaner 的痛点**：你装一个叫 "Claude" 的 web app，按 normalized name 匹配，会把 `~/.claude` 当残留删了——直接干掉 Claude CLI 的全部配置和会话历史。PureMac 把这个血的教训固化成一张编译期黑名单 + 不扫 `$HOME` 根。

**你们当前没有任何 dotfile 保护**。这条非常值得抄。

## 2.2 bundle ID 锚定匹配（防规则注入）

[AppPathFinder.swift#L131-L141](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/AppPathFinder.swift#L131-L141)：

```swift
/// Anchored check for whether `self.normalizedBundleID` belongs to the
/// family identified by `conditionBundleID`. Accepts exact equality,
/// ".child" extension, or "parent." suffix - rejects a bundle ID that
/// merely contains the condition string as a substring. This prevents
/// `com.evil.jetbrainsapp` from hijacking the `jetbrains` rule.
private func bundleIDMatchesCondition(_ conditionBundleID: String) -> Bool {
    if normalizedBundleID == conditionBundleID { return true }
    if normalizedBundleID.hasPrefix(conditionBundleID + ".") { return true }
    if normalizedBundleID.hasSuffix("." + conditionBundleID) { return true }
    return false
}
```

Pearcleaner 用的是 `cached.formattedBundleId.contains(condition.bundle_id)`——纯 substring 匹配。**这意味着恶意 app 把 bundle id 起成 `com.evil.jetbrainsapp` 就能命中 Pearcleaner 的 `jetbrains` 规则，强行把自己绑到 JetBrains 的 forceIncludePaths 上**。PureMac 把这个洞补了。

## 2.3 孤儿扫描的"白名单 root + 黑名单 fragment"双层安全

[OrphanSafetyPolicy.swift](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Utilities/OrphanSafetyPolicy.swift)：

```swift
static let allowedRoots: [String] = [
    "\(home)/Library/Caches", "\(home)/Library/Logs",
    "\(home)/Library/Saved Application State", "\(home)/Library/HTTPStorages",
    "\(home)/Library/WebKit", "\(home)/Library/Application Support/CrashReporter",
    "/Library/Caches", "/Library/Logs",
]
private static let blockedFragments: [String] = [
    "/Library/Preferences", "/Library/Containers", "/Library/Group Containers",
    "/Library/Application Scripts", "/Library/LaunchAgents", "/Library/LaunchDaemons",
    "/Library/Keychains", "/Library/Mail", "/Library/Safari", "/Library/Messages",
    "/Library/Calendars", "/Library/Accounts", "/Library/Mobile Documents", "/Library/CloudStorage",
]
```

孤儿扫描（反向扫描）特别危险：它扫出来的"残留"可能其实是另一个**还在用的 app** 的数据。PureMac 用了**白名单 + 黑名单双层**机制：

1. 白名单：孤儿删除**只能**落在 `Caches/Logs/HTTPStorages/WebKit/CrashReporter` 这些"易失数据目录"——Preferences、Containers、Group Containers 等"持久状态目录"不在白名单，孤儿扫描发现了也只展示不删。
2. 黑名单：即使命中白名单 root，路径里如果包含 `/Library/Preferences`、`/Library/Mail`、`/Library/Keychains` 等敏感 fragment，仍然拒绝。

 Pearcleaner 的反向扫描只有 [isRelatedToInstalledApp + conditions](file:///home/a16/macos/mvp/Pearcleaner/Pearcleaner/Logic/ReversePathsFetch.swift#L109-L142) 一层过滤，**没有这层"白名单 root"硬约束**，理论上能扫出 `~/Library/Preferences/xxx.plist` 当孤儿删掉。

## 2.4 删除时的 TOCTOU 双重校验（ CleaningEngine ）

[CleaningEngine.swift#L26-L62](file:///home/a16/macos/mvp/PureMac/PureMac/Services/CleaningEngine.swift#L26-L62)：

```swift
let resolvedURL = itemURL.resolvingSymlinksInPath()
let resolved = resolvedURL.path
guard isSafeToDelete(resolvedPath: resolved) else { continue }
// Narrow the TOCTOU window: re-resolve right before the delete
let reResolved = URL(fileURLWithPath: item.path).resolvingSymlinksInPath().path
guard reResolved == resolved else {
    // "Aborting delete: path resolution changed between check and unlink"
    continue
}
try fileManager.removeItem(at: resolvedURL)
```

[isSafeToDelete](file:///home/a16/macos/mvp/PureMac/PureMac/Services/CleaningEngine.swift#L150-L173) 也用"trailing `/` 防 sibling 撞名"——和 Lemon 一致的细节，但 PureMac 多了 **re-resolve 校验**：在检查和删除之间再 resolve 一次符号链接，不一致就 abort。这是防 symlink race 的硬功夫，Pearcleaner/Mole/Burrow 都没有。

---

# 三、PureMac 不如其他项目的地方

| 维度 | PureMac 弱在哪 | 应该参考谁 |
|---|---|---|
| **Sibling guard**（Xcode/Xcode-beta 共享数据保护）| 没有 | Mole [uninstall_live_bundle_has_other_install](file:///home/a16/macos/mvp/Mole/lib/uninstall/batch.sh#L980-L1130) |
| **Brew cask `--zap` 路由** | 没有 | Mole [brew.sh](file:///home/a16/macos/mvp/Mole/lib/uninstall/brew.sh) |
| **LaunchDaemon plist 内容二次确认** | 只做 base bundle id 剥离 | Lemon [McSoftwareFileScanner#L338-L393](file:///home/a16/macos/lemon-cleaner/localPod/QMCoreFunction/QMCoreFunction/Classes/McUninstaller/McSoftwareFileScanner.m#L338-L393) |
| **Trash 路由**（用户可恢复） | 直接 `removeItem` | 你们 Mole 的 `trash` crate（对齐 [03 红线 4](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)）|
| **私有 API `PKReceipt`** | 没用（这点好）| Pearcleaner 用，违反 [03 红线 6](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)，不抄 |

---

# 四、对你们方案的增量更新

我上一轮给的方案是 **Mole 安全地基 + Lemon 矩阵骨架 + Pearcleaner 容器解析 + Burrow UI 分类**。PureMac 给这个方案补三个新组件：

## 4.1 新增 `lib/core/high_risk_dotpaths.rs`（编译期黑名单）

按 [03 红线 3](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md)（规则编译期内嵌），把 PureMac 的 `highRiskHomeDotPaths` 列表 + "不扫 bare `$HOME`" 的策略固化进 Rust：

```rust
// src-tauri/src/lib/core/high_risk_dotpaths.rs

/// 严禁扫描或删除的 home dotdir/dotfile。任何卸载残留扫描结果命中即丢弃。
/// 防止"名为 Claude 的 web app 把 ~/.claude 当残留删了"这类血的教训。
/// 对齐 PureMac Conditions.swift#highRiskHomeDotPaths。
pub const HIGH_RISK_HOME_DOTPATHS: &[&str] = &[
    ".claude", ".ssh", ".aws", ".gnupg", ".gpg", ".kube", ".docker",
    ".config", ".git", ".gitconfig", ".git-credentials", ".netrc",
    ".npmrc", ".yarnrc", ".pnpmrc", ".pip", ".pypirc",
    ".rbenv", ".pyenv", ".nvm", ".cargo", ".rustup", ".gem",
    ".local", ".password-store", ".mozilla", ".wine",
    ".vscode", ".vim", ".viminfo",
    ".zshrc", ".zsh_history", ".bash_history", ".bashrc", ".bash_profile", ".profile",
];

/// 卸载残留扫描时，根 $HOME 不参与扫描（只扫 ~/Library / ~/Documents 等子目录）。
/// 对齐 PureMac Locations.swift 的设计：避免 normalized name 误匹配 dotfile。
pub const HOME_ROOT_NEVER_SCANNED: bool = true;

pub fn is_high_risk(path: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    for dot in HIGH_RISK_HOME_DOTPATHS {
        let full = format!("{home}/{dot}");
        if path == full || path.starts_with(&format!("{full}/")) {
            return true;
        }
    }
    false
}
```

在你们 `lib/uninstall/scanner.rs` 输出残留列表前调一次 `is_high_risk` 过滤；在 `batch.rs` 执行删除前再调一次兜底。

## 4.2 新增 `lib/core/bundle_id_anchor.rs`（锚定匹配）

抄 PureMac 的 [bundleIDMatchesCondition](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/AppPathFinder.swift#L131-L141)，用在 `embedded_rules.rs` 的 per-app 特殊规则匹配上，防恶意 bundle id 劫持规则：

```rust
pub fn bundle_id_matches_condition(app_bundle_id: &str, condition_bundle_id: &str) -> bool {
    let app = normalize(app_bundle_id);
    let cond = normalize(condition_bundle_id);
    if app == cond { return true; }
    if app.starts_with(&format!("{cond}.")) { return true; }
    if app.ends_with(&format!(".{cond}")) { return true; }
    false
}
```

## 4.3 新增 `lib/uninstall/orphan_safety.rs`（孤儿扫描白名单 root）

把 PureMac 的 [OrphanSafetyPolicy](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Utilities/OrphanSafetyPolicy.swift) 翻译成 Rust，用在你们未来要做的"孤儿残留 Tab"上：

```rust
const ORPHAN_ALLOWED_ROOTS: &[&str] = &[
    "~/Library/Caches", "~/Library/Logs", "~/Library/Saved Application State",
    "~/Library/HTTPStorages", "~/Library/WebKit",
    "~/Library/Application Support/CrashReporter",
    "/Library/Caches", "/Library/Logs",
];

const ORPHAN_BLOCKED_FRAGMENTS: &[&str] = &[
    "/Library/Preferences", "/Library/PreferencePanes",
    "/Library/Containers", "/Library/Group Containers",
    "/Library/Application Scripts", "/Library/LaunchAgents", "/Library/LaunchDaemons",
    "/Library/PrivilegedHelperTools", "/Library/Keychains",
    "/Library/Mail", "/Library/Safari", "/Library/Messages",
    "/Library/Calendars", "/Library/Accounts",
    "/Library/Mobile Documents", "/Library/CloudStorage",
];

pub fn is_safe_orphan_candidate(path: &str) -> bool {
    if crate::core::high_risk_dotpaths::is_high_risk(path) { return false; }
    let lower = path.to_lowercase();
    let ok = ORPHAN_ALLOWED_ROOTS.iter().any(|r| {
        let root = expand_tilde(r).to_lowercase();
        lower.starts_with(&format!("{}/", root))
    });
    if !ok { return false; }
    if ORPHAN_BLOCKED_FRAGMENTS.iter().any(|f| lower.contains(&f.to_lowercase())) {
        return false;
    }
    let name = std::path::Path::new(path).file_name()
        .and_then(|s| s.to_str()).unwrap_or("");
    if name.starts_with("com.apple.") || name == ".globalpreferences.plist" {
        return false;
    }
    true
}
```

## 4.4 在 `lib/core/file_ops.rs` 加 TOCTOU 双重 resolve

抄 PureMac [CleaningEngine](file:///home/a16/macos/mvp/PureMac/PureMac/Services/CleaningEngine.swift#L26-L62) 的双重 resolve，**在 trash crate 之前**加一层：

```rust
pub fn safe_remove(path: &Path) -> Result<(), String> {
    let resolved = std::fs::canonicalize(path).map_err(...)?;
    validate_path_for_deletion(&resolved)?;
    // TOCTOU 窗口收窄：删前再 resolve 一次，必须一致
    let re_resolved = std::fs::canonicalize(path).map_err(...)?;
    if re_resolved != resolved {
        return Err("path resolution changed between check and unlink".into());
    }
    trash::delete(&resolved).map_err(...)
}
```

---

# 五、最终四项目分工更新

把 PureMac 加进参考项目矩阵：

| 项目 | 在你们 MoleStudio2 中的角色 | 抄什么 |
|---|---|---|
| **Mole** | 算法/扫描清理逻辑的权威 + **安全地基** | sibling guard、trash 路由、brew cask --zap、敏感数据检测、launchctl bootout |
| **Lemon** | UI/UX 交互基准 + **扫描矩阵骨架** | 分类×路径×选项矩阵、CrashReporter 后缀正则、LaunchDaemon plist 内容二次确认、路径父子去重 |
| **Burrow** | UI Kind 分类 + 流程参考 | Auto/Review 分类、Clear Data、Input Method 检测 |
| **Pearcleaner** | 容器解析 + 反向扫描 | Container UUID（MCMMetadataIdentifier）、Group Container、Spotlight 补漏、三档灵敏度 |
| **PureMac** | **安全加固层** | highRiskHomeDotPaths 黑名单、bundle id 锚定匹配、孤儿白名单 root、TOCTOU 双重 resolve、`$HOME` 不扫描 |
| **v1 MoleStudio** | Rust 底座来源 | 已实现的 `lib/` 复用 |

---

# 六、一句话总结

> **PureMac 是 Pearcleaner 的安全加固重写版**——它的覆盖面和 Pearcleaner 完全相同（同样能卸载干净），但它把 Pearcleaner 几个安全漏洞补了：**dotfile 黑名单**（防 "Claude" app 误删 `~/.claude`）、**bundle id 锚定匹配**（防恶意 app 劫持规则）、**孤儿白名单 root**（反向扫描只能删易失数据）、**删除前 TOCTOU 双重 resolve**（防符号链接竞态）。
>
> 你们 Tauri/Rust 栈推荐方案更新为：**Mole 安全地基 + Lemon 矩阵骨架 + Pearcleaner 容器解析 + PureMac 安全加固 + Burrow UI 分类**。PureMac 的四个安全补丁（high_risk_dotpaths.rs / bundle_id_anchor.rs / orphan_safety.rs / file_ops.rs TOCTOU）应该作为 P0 优先级，因为它们防的是"误删用户核心数据"这种灾难性事故，比"多扫几个残留"重要得多。

要不要我下一步把 PureMac 的 [Conditions.swift](file:///home/a16/macos/mvp/PureMac/PureMac/Logic/Scanning/Conditions.swift) 里的 **appConditions / skipConditions / skipDeepSearch / skipReverse** 全套规则（包括 Xcode/Zoom/Chrome/VSCode/JetBrains 等几十个 app 的特殊处理）翻译成你们 `embedded_rules.rs` 的 Rust 编译期常量？这是直接落地、不引入运行时外置规则、对齐 [03 红线 3](file:///home/a16/work/molestudio/.trae/rules/03_硬约束与红线.md) 的一次性可执行工作。

