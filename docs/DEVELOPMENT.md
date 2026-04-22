逐个解释你这几个 script，重点讲 Tauri 相关的。

---

## `"tauri": "tauri"`

**用途**：把 `tauri` CLI 暴露给 `pnpm`。

**为什么不直接全局装 `tauri`**：项目里的 `@tauri-apps/cli` 是**固定在 devDependencies 里的版本**，跟你的 `tauri` crate 版本对齐。全局装的 `tauri` CLI 版本可能不匹配，导致构建行为不一致。

**怎么用**：

```bash
pnpm tauri dev          # 等价于 tauri dev
pnpm tauri build        # 等价于 tauri build
pnpm tauri info         # 查看环境信息，排查问题用
pnpm tauri --help       # 看所有子命令
```

**日常**：这个命令本身不直接跑，它是下面所有 `tauri:*` 命令的基础。你可以理解为"通过 pnpm 调用项目锁定的 tauri CLI"。

---

## `"tauri:dev": "TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data pnpm tauri dev"`

**用途**：开发模式启动，但**把 Tauri 的运行时数据目录重定向到项目内**。

**什么是 Tauri 的"数据目录"**：Tauri 应用运行时会往系统标准位置写东西：

- **macOS**：`~/Library/Application Support/com.a16.mole/`
- 包括：`tauri-plugin-store` 的 KV 数据、`tauri-plugin-log` 的日志文件、WebView 的 localStorage/IndexedDB、你自定义的配置

**默认行为的问题**：你 dev 时产生的数据，和用户装正式版产生的数据，**混在同一个目录**。开发时清个数据、改个 key，可能把"正式版"的数据搞乱。

**这个命令做的事**：设 `TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data`，让 dev 模式的数据写到**项目目录内**。

```
mole/
├── src-tauri/
│   ├── tauri_dev_data/       ← dev 时的数据都在这
│   │   ├── logs/
│   │   ├── store.json
│   │   └── ...
│   └── ...
```

**好处**：

1. **开发数据和用户数据隔离**——你随便造，不影响正式版
2. **方便清理**——`rm -rf src-tauri/tauri_dev_data` 一键重置 dev 状态
3. **方便检查**——日志、store 文件就在项目里，不用去 `~/Library/Application Support/` 翻
4. **方便 gitignore**——把这个目录加进 `.gitignore`，不会误提交

**代价**：没有。唯一的"副作用"是每次 dev 启动时，Tauri 会在这个目录里初始化一遍数据，跟正式版互不干扰。

**注意**：`TAURI_DEV_DATA_DIR` 这个环境变量是**你项目自定义的**，不是 Tauri 官方的。Tauri 官方是 `TAURI_DEV_DATA_DIR`？其实 Tauri v2 官方支持的是 `TAURI_CONFIG`、`TAURI_DEV_HOST` 等，数据目录重定向需要你在 Rust 侧读这个环境变量然后调 `tauri::Builder::setup` 里的 `app.path().resolve()`。所以你项目里应该有对应的 Rust 代码处理它。

**日常**：**这是你开发时的默认命令**，写代码、跑测试都用它。

---

## `"tauri:dev:fast": "TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data RUSTFLAGS='-C link-arg=-fuse-ld=lld' pnpm tauri dev"`

**用途**：跟 `tauri:dev` 完全一样，但**额外启用 LLD 链接器**。

**什么是 LLD**：LLVM 的链接器，比 macOS 默认的 `ld64` **快 2-5 倍**。Rust 编译时间的大头往往在链接阶段，尤其你项目里有 `objc2`、`core-foundation` 这些需要链接原生框架的依赖。

**怎么工作**：`RUSTFLAGS='-C link-arg=-fuse-ld=lld'` 告诉 rustc "链接时用 lld 而不是 ld64"。

**前提**：

```bash
brew install llvm
```

装完后 lld 在 `/usr/local/opt/llvm/bin/ld64.lld`（Intel Mac）或 `/opt/homebrew/opt/llvm/bin/ld64.lld`（Apple Silicon）。

**关键**：你命令里写的是 `-fuse-ld=lld`，rustc 会去找 `ld.lld` 或 `ld64.lld`。如果 PATH 里没有，需要指定完整路径：

```bash
RUSTFLAGS='-C link-arg=-fuse-ld=/usr/local/opt/llvm/bin/ld64.lld'
```

或者把 `/usr/local/opt/llvm/bin` 加进 PATH。

**收益**：链接阶段从 10-20 秒降到 3-5 秒。**改一行 Rust 代码后重编，感知最明显。**

**代价**：

1. 需要装 LLVM（约 1.5G 磁盘）
2. 如果 LLD 版本和系统不兼容，可能链接失败——这时回退到 `pnpm tauri:dev` 即可
3. **首次编译不变快**，只有链接阶段受益

**日常**：**装了 LLVM 后，这是你开发时的首选命令**。`:fast` 不是"快速模式"，而是"用更快的链接器"。

---

## `"format": "prettier --write src && cargo fmt --manifest-path src-tauri/Cargo.toml"`

**用途**：一键格式化**前端 + Rust**。

**拆解**：

- `prettier --write src` → 格式化 `src/` 下所有前端文件（`.ts`、`.tsx`、`.css` 等）
- `cargo fmt --manifest-path src-tauri/Cargo.toml` → 格式化 Rust 代码

**`--manifest-path` 的作用**：告诉 cargo "去 `src-tauri/Cargo.toml` 找项目"，而不是当前目录。因为你的 `Cargo.toml` 在 `src-tauri/` 下，不在项目根目录。

**日常**：提交前跑一次，保证代码风格统一。也可以配 Git pre-commit hook 自动跑。

---

## `"format:rs": "cargo fmt --manifest-path src-tauri/Cargo.toml"`

**用途**：只格式化 Rust，不动前端。

**为什么不直接用 `cargo fmt`**：因为你在项目根目录，`cargo fmt` 会找不到 `Cargo.toml`。必须指定 `--manifest-path`，或者先 `cd src-tauri`。

**日常**：只改了 Rust 代码时用，比 `format` 快（不跑 prettier）。

---

## `"tauri:dev:mas": "tauri dev -- --no-default-features --features mas"`

**用途**：以 **Mac App Store 版本**的 feature 组合启动 dev。

**拆解**：

- `tauri dev` → 标准 dev 启动
- `--` → 分隔符，后面是**传给 cargo 的参数**，不是传给 tauri CLI 的
- `--no-default-features` → 关掉 `Cargo.toml` 里的 `default = ["full"]`
- `--features mas` → 启用 `mas` feature

**你的 `Cargo.toml` 里**：

```toml
[features]
default = ["full"]
full = []
mas = []
```

`full` 和 `mas` 是**互斥**的两种构建模式：

- **`full`**：官网下载版，功能完整。可以用 `macos-private-api`、可以做沙箱外的操作
- **`mas`**：App Store 版，**必须走沙箱**，不能用某些私有 API，删除文件必须用 `trash` 而不是直接 `unlink`

**为什么需要单独的命令**：App Store 审核对沙箱和 API 使用有严格要求。你开发时用 `full`，但**提交前必须用 `mas` 测一遍**，确保沙箱下没有权限问题、没有崩溃。

**日常**：

- 平时开发用 `pnpm tauri:dev`（`full` 模式）
- **提交 App Store 前，用这个命令跑一遍完整测试**

**注意**：如果你在 `full` 模式下写了依赖 `macos-private-api` 的代码，切到 `mas` 模式会**编译失败**（feature 不匹配）。所以你的代码里应该用 `#[cfg(feature = "full")]` 和 `#[cfg(feature = "mas")]` 做条件编译。

---

## `"tauri:dev:mas:fast": "RUSTFLAGS='-C link-arg=-fuse-ld=lld' tauri dev -- --no-default-features --features mas"`

**用途**：`tauri:dev:mas` + LLD 加速。

**注意**：这个命令**没有 `TAURI_DEV_DATA_DIR`**。对比一下：

```bash
tauri:dev:mas       →  tauri dev -- --no-default-features --features mas
tauri:dev:mas:fast  →  RUSTFLAGS='...' tauri dev -- --no-default-features --features mas
```

两个都没有 `TAURI_DEV_DATA_DIR`。这意味着 **MAS 模式下 dev 数据写到系统默认位置**（`~/Library/Application Support/com.a16.mole/`），而不是项目内。

**这可能是有意的**——MAS 模式要模拟真实沙箱环境，数据目录走系统默认更接近真实。但**如果你想隔离，应该也加上 `TAURI_DEV_DATA_DIR`**：

```json
"tauri:dev:mas:fast": "TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data_mas RUSTFLAGS='-C link-arg=-fuse-ld=lld' tauri dev -- --no-default-features --features mas"
```

用 `tauri_dev_data_mas` 单独一个目录，避免和 `full` 模式的数据混。

**日常**：提交 App Store 前，用这个命令测 MAS 版本。比 `tauri:dev:mas` 快，因为用了 LLD。

---

## `"tauri:build:mas": "tauri build -- --no-default-features --features mas"`

**用途**：**打包** MAS 版本，用于提交 App Store。

**和 `tauri:dev:mas` 的区别**：

- `dev` → 开发模式，热重载，不打包
- `build` → 生产构建，生成 `.app` 和 `.pkg`，可以上传 App Store Connect

**产出**：

```
src-tauri/target/release/bundle/
├── macos/
│   └── mole.app           ← 可执行的 App
└── pkg/
    └── mole_0.1.0_x64.pkg ← 上传 App Store 用
```

**前提**：

1. **代码签名**：你的 `tauri.conf.json` 里 `signingIdentity: null`，MAS 构建需要配好 Apple Developer 证书
2. **Entitlements**：`entitlements: null`，MAS 需要专门的沙箱 entitlements 文件
3. **Provisioning Profile**：App Store 分发需要

**这些配置你目前都是 `null`，说明 MAS 打包流程还没配好**。`tauri:build:mas` 现在跑可能会失败或产出无法上传的包。需要补：

```json
"macOS": {
  "signingIdentity": "Apple Development: your@email.com (XXXXXXXXXX)",
  "entitlements": "entitlements.mas.plist",
  "providerShortName": "YourTeamName"
}
```

**日常**：只在**准备发版**时跑。平时开发不用。

---

## 汇总表

| 命令 | 用途 | 何时用 | 前提 |
|---|---|---|---|
| `tauri` | 调用项目锁定的 tauri CLI | 间接用 | 无 |
| `tauri:dev` | **日常开发**，数据隔离 | 写代码时 | 无 |
| `tauri:dev:fast` | **日常开发 + LLD 加速** | 写代码时（装了 LLVM 后首选） | `brew install llvm` |
| `format` | 格式化前端 + Rust | 提交前 | prettier、rustfmt |
| `format:rs` | 只格式化 Rust | 只改 Rust 时 | rustfmt |
| `tauri:dev:mas` | 开发 MAS 版 | 提交 App Store 前测试 | 代码要支持 `mas` feature |
| `tauri:dev:mas:fast` | MAS 版 + LLD 加速 | 同上 | LLVM |
| `tauri:build:mas` | **打包** MAS 版 | 发版时 | 签名、entitlements 配好 |

---