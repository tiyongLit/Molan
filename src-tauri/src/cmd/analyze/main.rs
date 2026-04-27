//! `Mole/cmd/analyze/main.go` 的 CLI 入口（camelCase → snake_case）。
//! TUI：`run_tui_mode` 仅占位；JSON：调用 `json::run_json_mode`（完整实现）。

use molestudio_lib::cmd::analyze::json;
use std::env;
use std::io;
use std::io::Write;
use std::path::Path;

/// 与 Go `flag` 包一致：在**第一个非 flag 参数之前**解析 flag，之后全部视为位置参数（不再解析 `--json`）。
fn parse_args() -> (bool, Vec<String>) {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut json_mode = false;
    let mut positionals: Vec<String> = Vec::new();

    for a in args {
        if positionals.is_empty() {
            if a == "--json" {
                json_mode = true;
            } else if a.starts_with('-') {
                let display = if let Some(rest) = a.strip_prefix("--") {
                    format!("-{rest}")
                } else {
                    a.clone()
                };
                let usage_target = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "rmole-analyze".to_string());
                let mut stderr = std::io::stderr();
                let _ = writeln!(stderr, "flag provided but not defined: {display}");
                let _ = writeln!(stderr, "Usage of {usage_target}:");
                let _ = writeln!(stderr, "  -json");
                let _ = writeln!(stderr, "    \toutput analysis as JSON instead of TUI");
                let _ = stderr.flush();
                std::process::exit(2);
            } else {
                positionals.push(a);
            }
        } else {
            positionals.push(a);
        }
    }

    (json_mode, positionals)
}

/// Go `filepath.Abs`：尽量 `canonicalize`；路径不存在时退化为「当前工作目录 + 路径」（与 Go 不要求存在的行为更接近）。
fn filepath_abs(target: &str) -> Result<String, io::Error> {
    let p = Path::new(target);
    if p.exists() {
        return Ok(p.canonicalize()?.to_string_lossy().into_owned());
    }
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        env::current_dir()?.join(p)
    };
    Ok(joined.to_string_lossy().into_owned())
}

/// Go `runTUIMode`：第一版占位（Go 中的 Bubble Tea / prefetch 未移植）。
fn run_tui_mode(path: &str, is_overview: bool) {
    eprintln!(
        "rmole-analyze: TUI 模式尚未实现（path={path}, overview={is_overview}）。请使用 --json。"
    );
    std::process::exit(3);
}

fn main() {
    let (json_mode, positionals) = parse_args();

    let mut target = env::var("MO_ANALYZE_PATH").unwrap_or_default();
    if target.is_empty() {
        if let Some(first) = positionals.first() {
            target = first.clone();
        }
    }

    let (abs, is_overview) = if target.is_empty() {
        ("/".to_string(), true)
    } else {
        match filepath_abs(&target) {
            Ok(abs) => (abs, false),
            Err(e) => {
                eprintln!("cannot resolve {target:?}: {e}");
                std::process::exit(1);
            }
        }
    };

    if json_mode {
        json::run_json_mode(&abs, is_overview);
    } else {
        run_tui_mode(&abs, is_overview);
    }
}
