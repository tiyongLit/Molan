//! 对齐 `lib/manage/purge_paths.sh`。
//!
//! GUI 端不需要 `clear screen` / `sleep` / 颜色等 TUI 装饰,本模块只翻译数据/逻辑。

use std::path::Path;

use crate::clean::project::load_purge_config;
use crate::clean::purge_shared::{MOLE_PURGE_DEFAULT_SEARCH_PATHS, mole_purge_read_paths_config};
use crate::core::base::{ensure_user_file, home_dir};

/// 对齐 `purge_paths.sh:16` 中的 `PURGE_PATHS_CONFIG` 默认值。
pub fn purge_paths_config() -> String {
    format!("{}/.config/mole/purge_paths", home_dir())
}

/// 对齐 `purge_paths.sh:19-33` 中的 `ensure_config_template`。
///
/// 模板与 SH 第 21-29 行严格一致(注意 `# ~/Work/ClientB` 这一行不能漏)。
pub fn ensure_config_template() {
    let config_file = purge_paths_config();
    if !Path::new(&config_file).is_file() {
        ensure_user_file(&config_file);
        let template = "# Mole Purge Paths - Directories to scan for project artifacts\n# Add one path per line (supports ~ for home directory)\n# Delete all paths or this file to use defaults\n#\n# Example:\n# ~/Documents/MyProjects\n# ~/Work/ClientA\n# ~/Work/ClientB\n";
        let _ = std::fs::write(&config_file, template);
    }
}

pub fn manage_purge_paths() {
    ensure_config_template();

    let config_file = purge_paths_config();
    let display_config = config_file.replacen(&home_dir(), "~", 1);

    println!("Purge Paths Configuration\n");
    println!("Current Scan Paths:");

    let paths = load_purge_config();
    if !paths.is_empty() {
        for p in &paths {
            let display = p.replacen(&home_dir(), "~", 1);
            if Path::new(p).is_dir() {
                println!("  ✓ {display}");
            } else {
                println!("  ○ {display}, not found");
            }
        }
    }

    let custom_count = mole_purge_read_paths_config(&config_file).len();
    println!();
    if custom_count > 0 {
        println!("Using custom config with {custom_count} paths");
    } else {
        println!(
            "Using {} default paths",
            MOLE_PURGE_DEFAULT_SEARCH_PATHS.len()
        );
    }

    println!();
    println!("Default Paths:");
    for p in &MOLE_PURGE_DEFAULT_SEARCH_PATHS {
        let display = p.replacen('~', &home_dir(), 1);
        println!("  - {display}");
    }

    println!();
    println!("Config File: {display_config}");
    println!();

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vim".to_string());

    println!("Opening in {editor}...");
    println!("Save and exit to apply changes. Leave empty to use defaults.");
    println!();

    std::thread::sleep(std::time::Duration::from_secs(1));

    let _ = std::process::Command::new(&editor)
        .arg(&config_file)
        .status();

    println!();
    println!("✓ Configuration updated");
    println!("Run 'mo purge' to clean with new paths");
}
