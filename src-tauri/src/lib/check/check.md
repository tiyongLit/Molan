改动总览
新增文件
src/lib/check/configuration.rs — 对齐 all.sh:33-97 的 System Configuration 三项:

check_touchid_sudo: 读 /etc/pam.d/sudo* 检查 pam_tid.so,fallback 到 bioutil -r / uname -m == arm64 判断是否支持。未配置且支持时 export TOUCHID_NOT_CONFIGURED=true
check_rosetta: 仅 arm64 上检查 /Library/Apple/usr/share/rosetta/rosetta
check_git_config: 检查 git config --global user.name/user.email
提供 collect_configuration_report / generate_configuration_json_value / check_all_configuration_ansi
src/lib/check/security.rs — 对齐 all.sh:99-191 的 Security Status 四项:

check_filevault: 解析 fdesetup status,关闭时 export FILEVAULT_DISABLED=true
check_firewall: 先检测 6 个第三方防火墙(Little Snitch / LuLu / Radio Silence / Hands Off! / Murus / Vallum),再回落到 socketfilterfw --getglobalstate。进入函数即 remove_var,关闭时 export FIREWALL_DISABLED=true
check_gatekeeper: spctl --status,关闭时 export GATEKEEPER_DISABLED=true,启用时 remove_var
check_sip: csrutil status(对齐 SH:不 export env)
修改 src/lib/check/system_health.rs
函数	修复点
check_disk_space_line	新增 export DISK_FREE_GB(整数 GB,对齐 SH:594)
check_cache_size_line	新增 export CACHE_SIZE_GB(保留一位小数字符串,对齐 SH:728);简化原先 cache_int > 10 || cache_int > 5 冗余条件为 cache_int > 5
check_macos_update_line	新增 export MACOS_UPDATE_AVAILABLE(true/false,对齐 SH:456);判定改为 软件清单非空 AND (summary 非空 OR 文本匹配 macos/security),新增 is_macos_software_update_text 辅助函数对齐 SH:244-255
check_brew_outdated_line	完全重写:新增 ~/.cache/mole/brew_updates TTL 600s 文件缓存(对齐 SH:343-352);双线程并发跑 formula/cask outdated(对齐 SH:369-378);两侧都成功才写缓存(对齐 SH:396-399);新增 export BREW_FORMULA_OUTDATED_COUNT / BREW_CASK_OUTDATED_COUNT / BREW_OUTDATED_COUNT,供 manage/update.rs 直接读取
get_softwareupdate_list	修复 OnceLock bug:原实现一旦写入空字符串就永久卡死。改为 OnceLock<Mutex<Option<String>>>,失败时回退到旧文件缓存(对齐 SH:305-310 行)
修改 src/lib/check/all.rs
CheckReport 新增 configuration: Value / security: Value 字段
collect_check_report_with_options 调用 configuration::generate_configuration_json_value() 与 security::generate_security_json_value()
check_mole_update 进入函数即 export MOLE_UPDATE_AVAILABLE=false(对齐 SH:484);命中 UpdateAvailable 时 export =true(对齐 SH:530)
修改 src/lib/check/mod.rs
新增 pub mod configuration; pub mod security;。

下游影响验证
manage/autofix.rs 与 manage/update.rs 期望读取的 env 现在都被正确写入:

Env 变量	写入位置
TOUCHID_NOT_CONFIGURED	configuration::check_touchid_sudo
FILEVAULT_DISABLED	security::check_filevault
FIREWALL_DISABLED	security::check_firewall
GATEKEEPER_DISABLED	security::check_gatekeeper
MACOS_UPDATE_AVAILABLE	system_health::check_macos_update_line
MOLE_UPDATE_AVAILABLE	all::check_mole_update
DISK_FREE_GB	system_health::check_disk_space_line
CACHE_SIZE_GB	system_health::check_cache_size_line
BREW_*_OUTDATED_COUNT	system_health::check_brew_outdated_line
之前 manage 端这些分支基本都是死代码,现在 GUI 调一次 check 后立刻就能让 autofix/update 给出有效建议。
