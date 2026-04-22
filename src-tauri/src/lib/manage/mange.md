对齐完成。manage/ 4 个文件全部通过 cargo check,无 lint 错误。其他 cargo check 错误来自仓库里别处文件（缺 crossbeam_channel / sysinfo / rayon 等 crate），与本次改动无关。

改动摘要
whitelist.rs（按 whitelist.sh 翻译）
新增 default_whitelist_patterns() / default_optimize_whitelist_patterns()，对齐 base.sh:88-118
新增 WHITELIST_CONFIG_OPTIMIZE_LEGACY 常量，对齐 whitelist.sh:15
save_whitelist_patterns 头部文本改为 SH 第 41/44 行的版本（含默认保护项说明），按 patterns_equivalent 去重
load_whitelist 补齐 legacy 文件读取 + 自动迁移（SH 199-202 / 238-241）+ 配置不存在时回退默认值（SH 214-218）+ 去重
get_all_cache_items() 从 ~35 项扩展到 76 项（含括号说明），与 SH 79-155 行严格一致
get_optimize_whitelist_items() 显示名补齐括号说明，与 SH 161-173 完全一致
跳过 TUI-only 的 manage_whitelist_categories（已由前端组件承担）
autofix.rs（按 autofix.sh 翻译）
show_suggestions 增加 MOLE_SECURITY_FIXES_SHOWN 跳过逻辑（SH 13-16）
新增三类健康建议：CACHE_SIZE_GB > 5、BREW_HAS_WARNINGS=true、DISK_FREE_GB < 50 与 cache_small 联动（SH 44-62）
通过 unsafe { std::env::set_var(...) } 导出 HAS_AUTO_FIX_SUGGESTIONS，对齐 SH 第 91 行
TouchID 修复改用 SH 第 155 行强调的 /usr/bin/sed 绝对路径，避免被 Homebrew gnu-sed 替换破坏 -i '' 语法
perform_auto_fix 完成后导出 AUTO_FIX_SUMMARY / AUTO_FIX_DETAILS，对齐 SH 180-191
TTY-only 的 ask_for_auto_fix 仍返回 false（GUI 由前端弹窗代替）
purge_paths.rs（按 purge_paths.sh 翻译）
模板补齐 # ~/Work/ClientB 一行（SH 28 行），与 SH 21-29 严格一致
用 ensure_user_file 替代手写 create_dir_all，对齐 SH prepare_purge_config_path
删掉未使用的 std::io::prelude::*
update.rs（按 update.sh 翻译）
新增 format_brew_update_label()（SH 32-36 兼容函数）
修正 format_brew_update_detail() 不再带 Homebrew, 前缀，改读 BREW_*_OUTDATED_COUNT 环境变量，与 SH 8-29 行为一致
populate_brew_update_counts_if_unset() 改用 run_with_timeout_capture(8.0, ...) 加 8s 超时（SH 55-56），并把结果写回 BREW_FORMULA_OUTDATED_COUNT / BREW_CASK_OUTDATED_COUNT / BREW_OUTDATED_COUNT（SH 62-64）
新增 brew_has_outdated(kind)（SH 67-76）
ask_for_updates() 完整检查 brew/appstore/macos/mole 四种来源（SH 80-130），TTY 确认部分由前端承担
perform_updates() 用 command -v mole 解析二进制路径（更接近 SH 第 142 行），命中后调用 reset_mole_cache()
删除 Bash 中不存在的 clean_mole_cache，reset_mole_cache 改为只清 ~/.cache/mole/mole_version（对齐 lib/check/all.sh:219-221 的 SH 真源）

mod.rs
未改动（仅模块声明）。

需要我接着做的话，还可以加个最小的单元测试集（例如 format_brew_update_detail 的 0/单 formula/单 cask/混合用例对 SH 输出做 bit-for-bit 对照），但既然你说"翻译就好不要加功能"，我就停在这里了。
