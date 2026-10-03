<div align="center">
  <h1>Molan</h1>
  <p><em>🐹 A native macOS cleanup & optimization app — deep cleaning, smart uninstall, system maintenance, and disk insights, with a GUI built for humans.</em></p>
</div>

<p align="center">
  <img src="https://img.shields.io/badge/platform-macOS_12%2B-blue?style=flat-square&logo=apple" alt="Platform">
  <img src="https://img.shields.io/badge/built_with-Tauri_2_%2B_Rust-orange?style=flat-square" alt="Tauri">
  <img src="https://img.shields.io/badge/frontend-React_19_%2B_TypeScript-61dafb?style=flat-square&logo=react" alt="React">
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License">
</p>

<p align="center">
  <a href="https://github.com/tiyongLit/Molan"><img src="https://img.shields.io/badge/GitHub-repo-181717?style=flat-square&logo=github" alt="GitHub"></a>
  <a href="https://gitee.com/tiyong/Molan"><img src="https://img.shields.io/badge/Gitee-mirror-orange?style=flat-square&logo=git&logoColor=white" alt="Gitee"></a>
</p>

<p align="center">
  English | <a href="./README.zh-CN.md">简体中文</a>
</p>

> 💡 **Molan** is a GUI app for cleaning, uninstalling, optimizing, and understanding your Mac. The scanning and cleanup logic is a **complete Rust rewrite** of the beloved open-source CLI [`tw93/Mole`](https://github.com/tw93/Mole), rebuilt around a native desktop experience: visual categories, checkboxes, previews, confirmations, and one-click reclaim — no terminal required.

<!-- Screenshot placeholder: drop a PNG into docs/img/ and replace this comment -->

## Why Molan

The original Mole CLI proved that a single tool could replace CleanMyMac, AppCleaner, DaisyDisk, and iStat Menus — but it lives in a terminal. Most Mac users never open one.

Molan takes the same battle-tested rules and algorithms, rewrites the entire engine in Rust (no external binaries, no shell scripts), and wraps it in a polished Tauri GUI designed around how people actually clean a computer:

- **See first, act second.** Every scan is side-effect-free. Results are grouped into visual categories with sizes, file counts, and native file icons before you select anything.
- **Nothing is deleted without you.** Cleanup always moves files to the Trash, and every destructive action requires an explicit confirmation.
- **Enhanced for GUI users.** Where the CLI offers interactive TUI lists, Molan adds expandable directory trees, auto-selected vs. needs-review grouping, deletion history, tray dashboards, and smart leftover detection.

## Features

### 🧹 Deep Clean

Scan known-safe caches, logs, browser data, developer artifacts, device leftovers, and large files — grouped by category (System, Apps, Browsers & Cloud, Developer, Devices, Large Files). Expand any category to inspect individual paths, toggle items, protect what you want to keep, and clean with one click.

### 🗑 Smart Uninstall

Uninstall apps from `/Applications` together with the preferences, caches, containers, and launch agents they leave behind — with a safety net the CLI can't offer:

- **Residual detection** — when an app is moved to the Trash manually, Molan prompts to scan and clean its leftovers.
- **Auto selected vs. Needs review** — residual files are split into confidently-safe and review-first groups, so shared data is never removed silently.
- **Sibling-app protection** — apps sharing a bundle ID (e.g. Xcode and Xcode-beta) keep their shared data intact.
- **Clear Data Only** — reset an app's state without uninstalling it.
- **Deletion history** — every uninstall is recorded; reopen the Trash with one click.

### 🔄 App Updates, Startup Items & Orphans

Beyond uninstall, the same page manages:

- **Updates** — check app updates, including Homebrew-managed casks and formulae.
- **Startup items** — login items and launch agents/services, tagged by provenance (Homebrew / user / vendor / system), enable or disable them individually.
- **Orphans** — find leftover files from apps that are already gone, and move them to the Trash.

### ⚡ Optimize

A guided maintenance pass with live performance diagnosis (high CPU, memory pressure, runaway processes) before anything runs. Network, disk, Spotlight, app databases, startup, and data services are analyzed first, previewed as a task list, then executed with a streaming log — unsafe or unnecessary tasks are skipped with a reason.

### 📊 Disk Analyze

A visual disk explorer: scan any volume or folder with a parallel Rust walker, then drill into a **treemap** of usage. Top-20 largest files, path filtering, Quick Look, Reveal in Finder, and confirmed move-to-trash — the DaisyDisk workflow, built in.

Two size metrics, switchable per scan: **logical** (what Finder shows) and **physical** (actual disk usage, matching Mole's accounting).

### 📈 Live Status & Tray Dashboard

A menu-bar dashboard with real-time CPU, temperature, fan speed, memory (with per-process quit), disk, and network stats — plus a trash-size reminder that pops up when the Trash exceeds your threshold.

### ⚙️ Built for Daily Use

- **Native look** — real macOS file and app icons via a content-addressed icon registry, native alert dialogs, and a tray-resident workflow.
- **Auto-update** — signed in-app updates (Tauri updater, GitHub/Gitee dual source).
- **Launch at login**, configurable update-check cadence, and a full **i18n** layer (English, 简体中文, 繁體中文).

## Safety & Privacy

Molan is built around the principle that a cleaner must never become the mess:

- **Trash, never `rm`.** All deletions go to the system Trash so you can recover.
- **Scan ≠ delete.** Scanning is read-only; cleanup only happens after you select and confirm.
- **Rules are compiled in.** Cleanup rules ship embedded in the binary (`embedded_rules.rs`), auditable in Git — no remote rule downloads, no hot-updated behavior.
- **No external binaries.** The entire engine is native Rust — no bundled CLI, no shell-outs, no AppleScript. This is also what keeps the app clean-room for Mac App Store sandboxing.
- **Local only.** Nothing is uploaded. No telemetry, no accounts. Your disk contents never leave your Mac.

## Architecture

One Rust engine, one GUI, one distribution build.

```
├── src/                    # Frontend — React 19 + TypeScript (strict)
│   ├── layout/             #   App shell (Sidebar, Dock, Shell*)
│   ├── pages/              #   Home · Clean · Uninstall · Optimize · Analyze · Settings · Dashboard
│   ├── components/         #   UI primitives (Mole*), business components, motion parts
│   ├── hooks/              #   useTauri, useNativeIcon, ...
│   └── i18n/               #   en-US · zh-CN · zh-TW
└── src-tauri/              # Backend — Tauri 2 + Rust
    ├── src/lib/            #   Engine: clean · uninstall · optimize · core · manage · check · platform
    ├── src/cmd/            #   analyze (parallel disk walker) · status (sysinfo collector)
    ├── src/controllers/    #   Thin #[tauri::command] entry points
    └── src/embedded_rules.rs  # Compile-time cleanup rules
```

Design notes:

- **All logic in Rust.** Path rules, size accounting, protected lists, and container resolution mirror [`tw93/Mole`](https://github.com/tw93/Mole) — reimplemented, not embedded.
- **Heavy I/O off the main thread.** Blocking scans run in `spawn_blocking`; the disk walker parallelizes with `rayon` + work-stealing queues for second-level full-home scans.
- **Streaming progress.** Long tasks push Tauri events the frontend subscribes to — no polling.
- **Typed contract.** Every command and event name is registered in `src/constants/tauri-commands.ts` / `tauri-events.ts`, with shared data types in `src/types/mole.ts`.

## Quick Start

Molan targets **macOS 12+** (Intel & Apple Silicon). Development requires macOS with Xcode Command Line Tools, [Node.js](https://nodejs.org) 20+, [pnpm](https://pnpm.io), and a recent [Rust](https://rustup.rs) toolchain.

The source is hosted on both GitHub and Gitee — use whichever is faster for you:

```bash
# GitHub
git clone https://github.com/tiyongLit/Molan.git
# Gitee (mirror, faster in mainland China)
git clone https://gitee.com/tiyong/Molan.git

cd Molan
pnpm install
pnpm tauri:dev          # launch in dev mode (data dir isolated to the repo)
```

Build the production app:

```bash
pnpm build:mac                # both DMGs (Apple Silicon + Intel) → release/
pnpm build:mac:arm            # Apple Silicon (aarch64) only
pnpm build:mac:intel          # Intel (x86_64) only
pnpm tauri build              # plain Tauri build for the host architecture
```

`pnpm build:mac` produces versioned DMGs for both architectures and collects them into `release/`. The version is read from `package.json` (single source of truth) and synced to `tauri.conf.json` and `Cargo.toml` before building. Every build is **ad-hoc code-signed** (Tauri's `signingIdentity: "-"`) and verified by the build script before artifacts are collected.

**macOS permissions.** macOS identifies apps for privacy (TCC) permissions by code signature — an unsigned app cannot read protected resources like the Trash, no matter what permissions the user grants. The Trash-size reminder therefore requires **Full Disk Access** (System Settings → Privacy & Security → Full Disk Access): grant it once, and re-grant after each upgrade (remove the old entry, re-add it, then restart the app). Web-downloaded DMGs are not notarized: on first launch, use right-click → Open.

Contributors: run `pnpm format` before committing (prettier + rustfmt; also enforced by a pre-commit hook).

## Roadmap

- [ ] Mac App Store release (sandbox + security-scoped bookmarks)
- [ ] Developer ID signing & notarization (replacing ad-hoc signing, for stable permissions across upgrades and warning-free distribution)
- [ ] Security-scoped bookmark onboarding for expanded scan ranges
- [ ] Purge (project build artifacts) & Installer cleanup GUI
- [ ] Custom rule import (strictly validated, user-authorized paths only)

## Acknowledgments

- [**tw93/Mole**](https://github.com/tw93/Mole) — the open-source CLI whose rules, categories, and algorithms this project is built on. Go star it.
- [**Lemon Cleaner** (Tencent)](https://github.com/Tencent/lemon-cleaner) — interaction and information-architecture reference for category panels and selection UX.

## License

Molan is licensed under the [MIT License](LICENSE).

The cleanup logic is inspired by [`tw93/Mole`](https://github.com/tw93/Mole) (GPL-3.0). Molan reimplements that behavior in Rust and does not link against or embed any of the original code.
