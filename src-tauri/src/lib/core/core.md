全部 21 项修复完成,lib/core/* 现在零 lint 错误、零类型错误。下面是这次修改的总结:

修改清单(按文件)
app_protection.rs
should_protect_path:补全了 _container_cache_path 通路 — /Data/Library/Caches/ 与 /Data/tmp/ 即便 bundle id 受保护也允许清理
should_protect_data:补齐 com.microsoft.* / com.docker.* / com.getpostman.* / com.insomnia.* / com.sublimetext.* / Cursor / Claude / ChatGPT / Ollama 等关键品牌的 startswith 早期分支
set_global_whitelist / is_path_whitelisted_global:新增全局白名单存储,GUI/CLI 启动注入后所有 safe_* 自动尊重
base.rs
wildcard_match:重写,新增 ? / [abc] / [a-z] / [!abc] 支持(Bash glob 全集)
ensure_user_dir / ensure_user_file:root 模式下用 SUDO_UID/SUDO_GID 把 ownership chown 回 invoking user
create_temp_file / create_temp_dir / mktemp_file:走系统 mktemp XXXXXX,失败 fallback 到 epoch+pid+atomic counter,杜绝并发碰撞
get_file_size / get_file_mtime:补 stat -L fallback
is_ansi_supported:加上 isatty(stdout) 真检查
common.rs
mole_normalize_path:保留根 /
mole_path_identity:严格校验 dev:inode 格式,加 stat -L fallback
update_via_homebrew:重做为 HomebrewUpdateOutcome 枚举,带 already-installed / Updated / Error 三态,补回版本号探测、~/.cache/mole/version_check 清理
remove_apps_from_dock:新增 normalize_dock_target — 支持 ~/、相对路径 canonicalize、不存在的绝对路径透传
file_ops.rs
新增 MOLE_OK / MOLE_ERR_GENERIC 常量,加上原有 SIP/AUTH/READONLY 三个错误码
safe_remove_ex / safe_sudo_remove_ex / safe_remove_symlink_ex:返回 i32,包含 SIP / 认证失败 / 只读 / 信号中断的精细诊断
兼容包装 safe_remove / safe_sudo_remove / safe_remove_symlink 仍是 bool,所有现有调用点零修改
新增 permission_denied_count 全局计数器,GUI 收尾时可提示 "需要 Full Disk Access"
mole_delete:接受 precomputed_size_kb,trash 模式失败时每会话警告一次
_mole_move_to_trash:加 needs_sudo 参数,优先 trash CLI、再 osascript 一次性 Finder 调用
_mole_move_to_trash_batch:真批量 — 一次 trash a b c d 或 一次 osascript,大批量卸载时 GUI 不再卡顿
validate_path_for_deletion:接入 should_protect_path 桥接,符号链接目标做 canonicalize / 组件级规范化
safe_find_delete / safe_sudo_find_delete:迭代时同时过 should_protect_path 和 is_path_whitelisted_global,sudo 版本拒绝软链接基目录
log.rs
log_info / log_success / log_warning / log_error:MO_DEBUG=1 时双写到主日志 + 调试日志
log_error:加 ICON_ERROR 前缀
debug_log / debug_operation_* / debug_file_action / debug_risk_level:同时输出到 stderr(实时反馈)
debug_file_action:格式从 path, age 改为 * path, size, age days old(完全对齐 SH)
log_operation_session_end:size 走 bytes_to_human 输出 "1.23GB" / "0B"
log_system_info:加 SYS_INFO_LOGGED 哨兵 + 启动时 truncate 调试日志
sudo.rs
will_need_sudo(operations: &[&str]) -> bool:语义改回操作分类(对齐 SH 接口)
register_sudo_cleanup:用 signal-hook 在独立线程监听 SIGINT/SIGTERM/SIGHUP → stop_sudo_session → 进程退出
_request_password:加 TtyGuard,save/restore termios,提示走 tty 设备
_start_sudo_keepalive:重试逻辑 + 父进程 kill -0 $$ 监控
is_clamshell_mode:严格匹配 "AppleClamshellState" = Yes,不再被 NotKey: Yes 误命中
timeout.rs
新增 Perl helper fallback(setsid + 进程组 SIGTERM/SIGKILL,对齐 SH 中段实现)
Rust fallback 也用 pre_exec(setsid) + kill(-pgid, SIG),杜绝 orphan child
MO_DEBUG=1 输出对齐 SH [TIMEOUT] 标签
新增 run_with_timeout_capture(secs, cmd, args) -> Option<String>,GUI 后端短超时调用 mdfind/pkgutil 用
bundle_resolver.rs
mdfind 走 run_with_timeout_capture(2.0, ...) — Spotlight 索引未就绪时 GUI 不再卡顿
pkg_receipts.rs
_mole_pkg_receipt_app_root:严格 .app 段匹配,不再误吃 foo.appdata.json
pkg_receipt_nonstandard_app_paths:pkgutil --pkgs 套 MOLE_PKG_RECEIPT_LIST_TIMEOUT(默认 3s),整体扫描套 MOLE_PKG_RECEIPT_SCAN_TIMEOUT(默认 8s),单个 --files 调用 5s
ui.rs
get_display_width(&str):Unicode 范围精确版本(Hangul / CJK A-F / Hiragana / Katakana / Fullwidth / 表情等)
新增 char_width + truncate_by_display_width(尾部追加 …)
format_last_used_summary:修正 month(s) ago 顺序
core/mod.rs
新增 init():统一入口,执行 prepare_mole_tmpdir + rotate_log_once + log_system_info + register_sudo_cleanup,幂等
新增 set_whitelist(patterns):GUI 加载白名单后调一次
Cargo.toml
新增直接依赖 libc = "0.2" / signal-hook = "0.3"(已有传递引入,显式化)
验证
cargo check --lib 中 lib/core/ 模块零 error / 零 warning
现有 29 个错误全部来自 cmd/、lib.rs、scanner 等其它模块的预存依赖缺失(rayon / sysinfo / crossbeam_channel / 缺失的 scanner 模块),与本次重构无关
GUI 后端调用方建议在 tauri::Builder::setup 中加这两行,后端命令就能直接复用 SH 那套语义:

crate::lib::core::init();
crate::lib::core::set_whitelist(load_whitelist_patterns_from_disk());
