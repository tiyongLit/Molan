# Molan 开发与发版指南

> 本指南面向"很久以后再回来发版"的场景：照着第四节操作即可完成一次发布。
> 最后校准：2026-10（1.0.0-alpha.2 发布周期）。

---

## 一、项目现状

- macOS 清理 / 优化 GUI 应用（React + Tauri 2 + Rust）。
- **发行渠道只有一个：官网完整版**（Gitee 分发 + 应用内自更新）。
- MAS（App Store 沙箱版）**已搁置**：代码中 `is_mas_build` / `open_appstore` 分支保留，仅保持可编译，不投入维护；构建命令中没有任何 MAS 相关项。

## 二、环境准备（新电脑接入）

1. 基础依赖：Node.js + pnpm、Rust（rustup）、Xcode Command Line Tools。
2. 添加双架构编译目标（交叉编译必需）：

   ```bash
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```

3. 拉代码并安装依赖：

   ```bash
   git clone https://gitee.com/tiyong/Molan.git
   cd Molan
   pnpm install
   ```

4. 配置签名私钥（发版必需）：从已有机器安全拷贝 `~/.molan-key.txt`（见第三节），并 `chmod 600 ~/.molan-key.txt`。
   - 只写代码不发版的话，第 4 步可以跳过。

## 三、签名密钥管理（关键，务必读完）

> 本节讲的是**更新包签名**（Ed25519，给应用内自更新验签用）。macOS 系统级的**代码签名**（决定完全磁盘访问等隐私授权能否生效）是另一件事，见本节末尾「macOS 代码签名（ad-hoc，构建时自动）」。

### 概念对照

| 名字 | 是什么 | 存在哪 | 需要动吗 |
|---|---|---|---|
| 私钥 | 「盖章机」，给更新包签名 | `~/.molan-key.txt`（两台电脑同一份） | 基本不动，构建时脚本自动读取 |
| 公钥 | 「验章对照表」，App 用它验证更新包 | `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，构建时打进 App | 只在轮换密钥时改一次 |
| 密码 | 本密钥**无密码**（刻意简化） | 无 | 永远不用管 |

构建输出里出现 `签名密码：无（私钥未设密码，属正常）` 是正常提示，无需提供任何密码。

### 两台电脑（家里 + 公司）如何处理

**原则：两台机器必须使用完全相同的同一份私钥文件。**
已发布的 App 里内置了对应公钥；用另一把钥匙签名的更新包，用户端验签会失败。

新机器首次配置（3 步）：

1. 从已配置的机器拷贝 `~/.molan-key.txt` 到新机器同样位置。
   - 安全渠道：隔空投送（AirDrop）、密码管理器附件、加密 U 盘。
   - 不要走 git、聊天软件明文、邮件。
2. 收紧权限：`chmod 600 ~/.molan-key.txt`
3. 验证：在项目目录执行 `bash scripts/build-dmg.sh --dry-run`，应看到 `签名私钥：/Users/<用户名>/.molan-key.txt`。

日常纪律：

- 只「拷贝」私钥；**绝不在第二台机器上重新执行 `tauri signer generate`**（那会生成两把不同的钥匙，签出的包用户装不上）。
- 快速核对两台机器钥匙是否一致：两台分别执行 `shasum -a 256 ~/.molan-key.txt`，输出必须相同。
- **备份**：把私钥存在安全处（密码管理器 / 加密盘）。丢了就无法再发自更新，老用户只能手动重装新包。
- 私钥永不进 git、永不发到任何聊天 / 邮件里。

### 密钥轮换（仅当泄露 / 丢失时）

前提认知：已发布 App 内置的是旧公钥；轮换后**老用户需手动重装一次新包**，才能接上新钥匙的后续更新。

1. 生成新密钥（无密码、非交互）：

   ```bash
   CI=true pnpm tauri signer generate -w ~/.molan-key.txt --force
   ```

2. `chmod 600 ~/.molan-key.txt`
3. 打开 `~/.molan-key.txt.pub`，把**整个内容**替换到 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。
4. 把新私钥同步到另一台机器（替换旧文件），并按上节 3 步验证。
5. 提高一个版本号，按第四节发一版。

### macOS 代码签名（ad-hoc，构建时自动）

与「更新包签名」正交的另一件事：macOS 系统对 app 的**代码签名身份**——它是隐私授权（TCC）能否生效的前提。

- **为什么必需**：macOS 隐私保护按代码签名身份记录授权。app 完全未签名时，tccd 无法验证其身份（系统日志报 `-67062: Invalid Code Signature`），用户手动添加的「完全磁盘访问权限」也不会生效——曾实际导致生产包「废纸篓清理提醒」永远显示"无法读取废纸篓，请检查访问权限"（读取 `~/.Trash` 被拒 `EPERM`；应用日志特征：`[trash-watch] watch unavailable: Operation not permitted`）。
- **怎么做的**：`src-tauri/tauri.conf.json` → `bundle.macOS.signingIdentity: "-"`（Tauri 官方支持的 ad-hoc 伪身份）。每次构建自动对 app 做 ad-hoc 签名，dmg 内的 app 同样带签名。
- **防回归**：`scripts/build-dmg.sh` 在每个架构构建完成后强制 `codesign --verify`，未签名**直接中断出包**。正常构建会输出 `签名校验通过：Identifier=com.tiyong.molan Signature=adhoc`。
- **代价与边界（无 Apple Developer 账号阶段的取舍）**：
  - 用户侧：涉及隐私资源的功能（废纸篓提醒 / 清空废纸篓）需要一次性授权「系统偏好设置 → 安全性与隐私 → 隐私 → 完全磁盘访问权限」（macOS 13+ 为「系统设置 → 隐私与安全性」）。**每发一版新包（cdhash 变化），授权都要"移除旧条目 → 重新添加"一次**；授权后需完全退出并重启 app（Cmd+Q 真退出，仅关窗口 / 刷新界面无效）。
  - 分发侧：产物未经公证，从网络下载 dmg 首次打开会被 Gatekeeper 拦截，安装说明须提示**右键 → 打开**。
- **将来进阶**：购买 Apple Developer 账号后，把 `signingIdentity` 换成 Developer ID 证书（或设 `APPLE_SIGNING_IDENTITY` 环境变量）并配置公证（`APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID`），即可实现跨版本稳定授权 + 无拦截分发，代码零改动。

## 四、发版流程（核心章节）

### 发版前 1 分钟自查

- [ ] 两台机器先 `git pull`（确保代码最新、避免推送冲突）。
- [ ] 决定版本号：修改 `package.json` 的 `version`（往上加一位，如 `1.0.0-alpha.2` → `1.0.0-alpha.3`）。
      **`package.json` 是唯一版本事实源**；不要手改 `tauri.conf.json` / `Cargo.toml` 的版本，构建脚本会自动同步。
- [ ] 私钥在位：`bash scripts/build-dmg.sh --dry-run` 一眼确认。

### 第 1 步 · 构建（唯一命令）

```bash
pnpm build:mac          # 双架构（发版用这个）
# 单架构备选：pnpm build:mac:arm / pnpm build:mac:intel
```

脚本自动完成：同步版本号 → 注入私钥 + 签名预检（约 3 秒）→ 双架构编译（含 ad-hoc 代码签名 + 构建后强制校验）→ 收集产物 → 生成更新清单。

> 耗时参考：首次全量 10–30 分钟，增量会快一些。全程无交互，不要按 Ctrl-C；
> 签名若有问题会在开头 3 秒内报错，不会白等编译。

构建完成后 `release/` 目录（已 gitignore）内容：

| 文件 | 用途 | 传 Gitee？ |
|---|---|---|
| `Molan_<版本>_aarch64.app.tar.gz` | M 芯片自更新包 | 必须 |
| `Molan_<版本>_x64.app.tar.gz` | Intel 自更新包 | 必须 |
| `Molan_<版本>_aarch64.dmg` / `_x64.dmg` | 新用户安装包 | 建议 |
| `*.app.tar.gz.sig` | 签名文件（内容已写进 latest.json） | 不用传 |

> 更新清单不在本目录：构建脚本直接把清单写入 `update/latest.json`（唯一清单，随 Git 提交，见第 3 步）。

> 提示：产物自带 ad-hoc 代码签名（见第三节）。老用户升级后若用到废纸篓相关功能，需引导其授权「完全磁盘访问权限」：移除旧条目 → 重新添加 → 完全退出并重启 app；每轮升级都要重做一次。

### 第 2 步 · Gitee 建 Release 并上传

1. 打开 <https://gitee.com/tiyong/Molan/releases/new>
2. Tag 填**与 package.json version 完全一致**的版本号，如 `1.0.0-alpha.3`。
   - **不带 `v` 前缀**；填错会导致用户下载 404（latest.json 里的下载地址就是按这个 tag 拼的）
3. 从 `release/` 拖入 4 个文件（**文件名保持原样，不要改**）：两个 `.app.tar.gz`（必须）+ 两个 `.dmg`（建议）。
4. 发布说明（Release 描述）可顺手写本轮更新内容。

> 顺序要求：**必须先传完包，再进第 3 步**。

### 第 3 步 · 提交清单并推送

构建时清单已由脚本直接写入 `update/latest.json`（无需手动复制）：

```bash
git add update/latest.json
git commit -m "chore(release): 1.0.0-alpha.3"
git push
```

> 构建结束时脚本会把这几行打印出来，可直接复制（版本号记得替换）。
> 若本地还有其他未推送的历史提交，会一并推上去（正常）。

### 第 4 步 · 验证（push 后等约 1 分钟）

```bash
curl -sL https://gitee.com/tiyong/Molan/raw/master/update/latest.json | head -4
```

看到 `"version": "新版本号"` 即为通（Gitee raw 有约 60 秒 CDN 缓存，刚推送立刻查可能拿旧内容）。

App 内验证：安装新 dmg → 托盘齿轮 → 「检查更新」→ 应显示"已是最新"。

### 可选 · 完整更新闭环预演

想立刻验证"下载 → 验签 → 替换 → 重启"全链路：把 `update/latest.json` 的 `version` 临时改成一个**更高的假版本号**（`url` / `signature` 保持不动），push → App 内点检查更新 → 红点出现 → 走完更新流程 → 验证后把 `version` 改回真值再 push。

> 原理：更新插件只拿 `version` 做"远端 vs 当前"比较，不校验包内版本号，所以可以安全预演。

## 五、自更新是怎么工作的（30 秒原理）

```
App 内置公钥 + 更新源（配置在 tauri.conf.json → plugins.updater）
  │ 启动后静默检查 / 托盘齿轮手动「检查更新」
  ▼
请求 https://gitee.com/tiyong/Molan/raw/master/update/latest.json（GitHub 为备用源，暂未配置）
  │ 版本比较：远端 > 当前 → 有更新（齿轮亮红点）
  ▼
下载 Release 附件 .app.tar.gz → Ed25519 验签（与 App 内置公钥配对）→ 替换 App → 自动重启
```

改配置 / 发布时的关键约束：

- `latest.json` 的 `platforms` 键必须覆盖**所有已分发的架构**（`darwin-aarch64` / `darwin-x86_64`）。
  缺哪个架构的键，那个架构的客户端「检查更新」会**直接报错**（而不是显示无更新）。
- 各架构用户用的是各架构的更新包，文件名不同（`_aarch64` / `_x64`），别传错。
- 更新源为 Gitee 主源 + GitHub 备源（配置里预留了 GitHub 地址，但对应仓库 / latest.json 尚未建立；Gitee 正常时不会访问备源，当前单 Gitee 即可）。脚本已留 GitHub 口子：开通时设 `GITHUB_REPO` 重新生成清单，即可产出备源草稿 `release/latest.github.json`。
- 版本线备忘：`1.0.0-alpha.1` 为占位公钥构建，**永远收不到自更新**；`1.0.0-alpha.2` 起为真密钥构建，可接收后续所有更新。

## 六、常见问题速查

| 现象 | 原因 / 处理 |
|---|---|
| 构建报「未找到签名私钥」 | 本机没有 `~/.molan-key.txt` → 见第三节"新机器首次配置" |
| 构建报「签名预检失败——私钥与密码不匹配」 | 私钥文件损坏 / 换成了别的钥匙；或环境里残留了 `TAURI_SIGNING_PRIVATE_KEY(_PATH/_PASSWORD)` 变量；或存在旧的密码文件 `~/.molan-key.txt.password`（有内容就删除，当前密钥无密码） |
| 构建"卡住"很久没反应 | 正常，双架构 Rust 全量编译本来就慢；签名问题会在开头 3 秒报错，不会卡在这里 |
| 用户下载更新 404 | ① 包没上传 / 文件名被改；② tag 与 latest.json 里 URL 的 tag 不一致；③ 顺序反了（先推了 latest.json 后传包） |
| 检查更新报错（而非"已是最新"） | latest.json 缺当前架构的平台键 / latest.json 未推送 / 刚推完还没过 60 秒缓存 |
| 改完 update/latest.json 立刻测不到 | 等约 60 秒 CDN 缓存后再试 |
| `pnpm build:mac` 报 TS / 前端错误 | 先跑 `pnpm build` 本地复现并修复前端错误 |
| 构建报「产物缺少有效代码签名」 | `tauri.conf.json` 的 `signingIdentity` 被改回 `null` 或签名未执行 → 恢复为 `"-"` 后重新构建 |
| 用户反馈「无法读取废纸篓，请检查访问权限」 | ① 用户装的是未签名旧版 → 让其升级到带签名的新包；② 未授权完全磁盘访问权限，或升级后未重做授权 → 引导：完全磁盘访问权限 → 移除 Molan → 重新添加 → Cmd+Q 重启 app（详见第三节结尾） |

## 七、日常开发命令速查

| 命令 | 用途 |
|---|---|
| `pnpm tauri:dev` | **日常开发**（运行时数据隔离到 `src-tauri/tauri_dev_data/`，不污染正式版数据） |
| `pnpm dev` / `pnpm preview` | 纯前端 Vite（无 Tauri IPC，仅调 UI 时用） |
| `pnpm build` | 前端类型检查 + 打包 dist（tauri build 会自动调用，一般不用手跑） |
| `pnpm format` / `pnpm format:rs` | 格式化前端 + Rust / 仅 Rust（pre-commit 钩子会自动兜底） |
| `bash scripts/build-dmg.sh --dry-run` | 只检查：同步版本号 + 显示私钥状态与构建计划，不编译 |

> 说明：所有 `pnpm tauri ...` 都是调用项目内锁定版本的 CLI，不要全局安装 `tauri`（版本不匹配会导致构建行为不一致）。
> `TAURI_DEV_DATA_DIR` 为本项目自定义的隔离变量（Rust 侧读取），不是 Tauri 官方变量。

## 八、关键文件索引

| 路径 | 作用 |
|---|---|
| `scripts/build-dmg.sh` | 构建主脚本：版本同步 + 私钥注入 + 签名预检 + 双架构构建 + 代码签名校验 + 产物收集 + 生成清单 |
| `scripts/gen-latest-json.sh` | 单独重新生成 `update/latest.json`（扫描 release/ 产物，无需重新编译；设 `GITHUB_REPO` 可额外产出 GitHub 备源草稿） |
| `update/latest.json` | 更新清单（推送后生效；App 检查更新读取的就是它） |
| `src-tauri/tauri.conf.json` | 更新源 endpoints、公钥 pubkey、`createUpdaterArtifacts` 开关、`signingIdentity`（ad-hoc `"-"`）、版本号（自动同步） |
| `src-tauri/src/lib/manage/app_version.rs` | 自更新后端逻辑（检查 / 下载 / 安装） |
| `src/hooks/useAppVersion.ts` | 自更新前端逻辑（红点 / 静默检查 / 安装进度） |
| `release/` | 本地产物目录（gitignore，同名文件直接覆盖） |
| `docs/自更新功能实现总结.md` | 自更新设计背景与实现细节（发版操作仍以本指南为准） |
