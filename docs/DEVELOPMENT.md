# DEVELOPMENT.md — 开发脚本说明

本文逐个解释 `package.json` 里的 `scripts`，重点是 Tauri 相关的。

---

## `"dev": "vite"`

**用途**：只启动前端 Vite dev server（`http://localhost:1420`），不带 Rust 后端。

**什么时候用**：纯调 UI 样式、不依赖 Tauri IPC 时。日常开发**不用它**，用下面的 `tauri:dev`（它会自动帮你拉起这个 dev server）。

---

## `"build": "tsc && vite build"`

**用途**：先跑 TypeScript 类型检查（`tsc`），再产出前端静态资源到 `dist/`。

**注意**：`tauri build` 会通过 `tauri.conf.json` 的 `beforeBuildCommand` 自动调用它，你一般不用手动跑。

---

## `"preview": "vite preview"`

**用途**：本地预览 `dist/` 产物，检查构建结果。同样不涉及 Tauri IPC。

---

## `"tauri": "tauri"`

**用途**：把 `tauri` CLI 暴露给 `pnpm`。

**为什么不全局装 `tauri`**：项目里的 `@tauri-apps/cli` 固定在 devDependencies，版本与 `tauri` crate 对齐。全局装的 CLI 版本可能不匹配，导致构建行为不一致。

**怎么用**：

```bash
pnpm tauri dev          # 等价于 tauri dev
pnpm tauri build        # 等价于 tauri build
pnpm tauri info         # 查看环境信息，排查问题用
pnpm tauri --help       # 看所有子命令
```

**日常**：这个命令本身不直接跑，它是下面 `tauri:*` 命令的基础。你可以理解为"通过 pnpm 调用项目锁定的 tauri CLI"。

---

## `"tauri:dev": "TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data pnpm tauri dev"`

**用途**：开发模式启动，并把 Tauri 的运行时数据目录重定向到项目内。

**什么是 Tauri 的"数据目录"**：应用运行时往系统标准位置写的东西：

- **macOS**：`~/Library/Application Support/com.tiyong.molestudio/`
- 包括：`tauri-plugin-store` 的 KV 数据、`tauri-plugin-log` 的日志文件、WebView 的 localStorage/IndexedDB、你自定义的配置

**默认行为的问题**：dev 时产生的数据和用户装的正式版数据混在同一目录。开发时清个数据、改个 key，可能把正式版数据搞乱。

**这个命令做的事**：设 `TAURI_DEV_DATA_DIR=./src-tauri/tauri_dev_data`，让 dev 数据写到项目目录内。

```
src-tauri/
└── tauri_dev_data/       ← dev 时的数据都在这
    ├── logs/
    ├── store.json
    └── ...
```

**好处**：

1. **开发数据与用户数据隔离** —— 随便造，不影响正式版
2. **方便清理** —— `rm -rf src-tauri/tauri_dev_data` 一键重置 dev 状态
3. **方便检查** —— 日志、store 文件就在项目里，不用去 `~/Library/Application Support/` 翻
4. **方便 gitignore** —— 加进 `.gitignore` 不会误提交

**注意**：`TAURI_DEV_DATA_DIR` 是**本项目自定义**的环境变量，不是 Tauri 官方的（Tauri 官方是 `TAURI_CONFIG`、`TAURI_DEV_HOST` 等）。数据目录重定向由 Rust 侧读取该环境变量后在 `setup` 里处理。

**日常**：**这是你开发时的默认命令**，写代码、跑测试都用它。

---

## `"format": "prettier --write src && cargo fmt --manifest-path src-tauri/Cargo.toml"`

**用途**：一键格式化前端 + Rust。

- `prettier --write src` → 格式化 `src/` 下前端文件（`.ts`、`.tsx`、`.css`、`.scss` 等）
- `cargo fmt --manifest-path src-tauri/Cargo.toml` → 格式化 Rust 代码

**`--manifest-path` 的作用**：告诉 cargo 去 `src-tauri/Cargo.toml` 找项目，而不是当前目录（`Cargo.toml` 不在仓库根）。

**日常**：提交前跑一次。也可以不跑 —— husky 的 `pre-commit` 钩子会通过 `lint-staged` 自动格式化暂存文件。

---

## `"format:rs": "cargo fmt --manifest-path src-tauri/Cargo.toml"`

**用途**：只格式化 Rust，不动前端。比 `format` 快（不跑 prettier）。只改了 Rust 代码时用。

---

## `"prepare": "husky"`

**用途**：`pnpm install` 后自动执行，安装 Git hooks。

**实际内容**：`.husky/pre-commit` 跑 `pnpm exec lint-staged`，按 `package.json` 的 `lint-staged` 配置：

- `src/**` 前端文件 → prettier
- `src-tauri/**/*.rs` → `rustfmt --edition 2024`

**日常**：装一次即生效，不用手动跑。

---

## 生产构建

项目只发**一个完整版**（官网分发），不维护 Mac App Store 沙箱版。构建直接用官方命令：

```bash
pnpm tauri build
```

两个都没有 `TAURI_DEV_DATA_DIR`。这意味着 **MAS 模式下 dev 数据写到系统默认位置**（`~/Library/Application Support/com.tiyong.molestudio/`），而不是项目内。

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
│   └── mole.app
└── dmg/  ·  zip/          ← 官网分发产物
```

**发布前提**（当前 `tauri.conf.json` 里 `signingIdentity` 与 `entitlements` 均为 `null`，发布前需补）：

1. Apple Developer ID 证书 + 公证（notarization）
2. 自更新签名密钥：`tauri signer generate -w ~/.tauri/mole.key`，公钥填进 `tauri.conf.json` 的 `plugins.updater.pubkey`（当前是占位符，必须替换）

---

## 汇总表

| 命令 | 用途 | 何时用 |
|---|---|---|
| `tauri:dev` | **日常开发**，数据目录隔离 | 写代码时 |
| `dev` / `preview` | 纯前端 Vite，无 IPC | 只调 UI 时 |
| `build` | 前端类型检查 + 打包 `dist/` | `tauri build` 会自动调用 |
| `tauri build` | **生产构建**（唯一发行版本） | 发版时 |
| `format` | 格式化前端 + Rust | 提交前（pre-commit 已兜底） |
| `format:rs` | 只格式化 Rust | 只改 Rust 时 |
