//! 磁盘分析常量定义。
//!
//! V2 展示口径对齐 lemon-cleaner 磁盘分析（localPod/LemonSpaceAnalyse）：
//! - 不再按目录名跳过/折叠（旧版 Mole Go 的 `skipSystemDirs`/`foldDirs` 已废弃）——
//!   每一级目录的全部条目（含隐藏文件、0 大小条目、symlink）都枚举并展示；
//! - 递归排除仅对齐 Lemon `LMFileScanTask.m` 的 3 处：`/System/Volumes`、`/Volumes`、
//!   路径含 `/private/tmp/msu-`；被排除目录仍展示为条目，只是不下钻（大小为 0）。
#![allow(non_upper_case_globals)] // 名称与 Go 标识符一一对应（Rust 用 snake_case）

use std::time::Duration;

pub const max_large_files: usize = 20;
pub const bar_width: usize = 24;
pub const large_file_warmup_min_size: u64 = 1 << 20;
pub const overview_cache_ttl: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const overview_cache_file: &str = "overview_sizes.json";

/// Go overview snapshot store budget（7cf9e382）：快照存储是单 JSON 文件、每次 save
/// 全量重写，所以既要限制长度也要限制写入频率：上限 1000 条、淘汰到低水位 900，
/// 目录未变时在 TTL/8 内跳过重写（refresh divisor）。
pub const overview_cache_max_entries: usize = 1000;
pub const overview_cache_keep_entries: usize = 900;
pub const overview_refresh_divisor: i32 = 8;
pub const max_concurrent_overview: usize = 8;
pub const cache_mod_time_grace: Duration = Duration::from_secs(30 * 60);
pub const cache_reuse_window: Duration = Duration::from_secs(24 * 60 * 60);
pub const stale_cache_ttl: Duration = Duration::from_secs(3 * 24 * 60 * 60);
pub const analyzer_cache_ttl: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Go `overviewDuIgnoreNames`：iCloud Drive FileProvider 会让遍历卡住数十秒，
/// overview 阶段跳过以减少延迟；用户后续可显式点进该目录分析。
pub const overview_du_ignore_names: &[&str] = &["Mobile Documents"];

/// 递归排除（对齐 Lemon `LMFileScanTask.m:255`）：精确路径不递归，但条目仍展示
/// （大小为 0）。避免重复统计系统卷、外置卷与 macOS 更新临时目录。
pub const no_recurse_exact: &[&str] = &["/System/Volumes", "/Volumes"];

/// 递归排除（对齐 Lemon `LMFileScanTask.m:255`）：路径前缀命中不递归，条目仍展示。
pub const no_recurse_prefix: &[&str] = &["/private/tmp/msu-"];

/// Bundle 叶子捷径（对齐 Lemon `LMFileScanManager.m:60` specialFileExtensions）：
/// 扩展名命中的目录在首扫时用 Spotlight 聚合大小叶子化（不递归内部），
/// 钻取时按需子树扫描。不扩 `.framework`（柠檬不叶子化 framework，保持口径一致）。
pub const bundle_leaf_extensions: &[&str] = &["app", "bundle", "simruntime"];

pub const skip_extensions: &[&str] = &[
    ".go", ".js", ".ts", ".tsx", ".jsx", ".json", ".md", ".txt", ".yml", ".yaml", ".xml", ".html",
    ".css", ".scss", ".sass", ".less", ".py", ".rb", ".java", ".kt", ".rs", ".swift", ".m", ".mm",
    ".c", ".cpp", ".h", ".hpp", ".cs", ".sql", ".db", ".lock", ".gradle", ".mjs", ".cjs",
    ".coffee", ".dart", ".svelte", ".vue", ".nim", ".hx",
];

/// Go `spinnerFrames`
pub const spinner_frames: &[&str] = &["|", "/", "-", "\\", "|", "/", "-", "\\"];

pub const color_purple: &str = "\x1b[0;35m";
pub const color_purple_bold: &str = "\x1b[1;35m";
pub const color_gray: &str = "\x1b[0;90m";
pub const color_red: &str = "\x1b[0;31m";
pub const color_yellow: &str = "\x1b[0;33m";
pub const color_green: &str = "\x1b[0;32m";
pub const color_blue: &str = "\x1b[0;34m";
pub const color_cyan: &str = "\x1b[0;36m";
pub const color_reset: &str = "\x1b[0m";
pub const color_bold: &str = "\x1b[1m";
