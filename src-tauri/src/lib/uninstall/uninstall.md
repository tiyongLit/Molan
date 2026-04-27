修改内容
brew.rs(对齐 brew.sh)
Bug / 危险行为

删除 brew_uninstall_cask 失败时的 sudo rm -rf $app_path 兜底——SH 完全没有这种行为,这条会绕过所有路径校验和 Trash 模式,可能误删用户数据。手工兜底交给 batch.rs 按 is_brew_cask_installed 三态码做。
对齐 SH

is_brew_cask_installed 改返回 CaskInstallState(三态:Installed=0 / NotInstalled=1 / Unknown=2)。
resolve_path 加 realpath 失败时 std::fs::canonicalize 兜底。
brew_uninstall_cask 完整翻译 SH 第 183-256 行:
按 app 体积选 timeout(<=5GB:300s,5-15GB:600s,>15GB:900s);
经 run_with_timeout 包裹;
SUDO_USER 非空 → sudo -u $SUDO_USER env ...;
始终设 HOMEBREW_NO_ENV_HINTS=1 HOMEBREW_NO_AUTO_UPDATE=1 NONINTERACTIVE=1;
退出码 124 立即失败、跳过验证;
验证 cask + app 双 gone 才算成功。
_extract_cask_token_from_path 校验首字符必须是字母数字。
_detect_cask_via_brew_list 走 eq_ignore_ascii_case + brew info 双重确认,与 SH grep -Fix 对齐。
新增 brew_autoremove_silent(对齐 batch.sh 第 962-968 行的后台 brew autoremove)。
batch.rs(对齐 batch.sh)
Bug / 安全修复

has_sensitive_data 不再返回 false,完整翻译 SH case 模式:.ssh/.gnupg/.aws/.kube/.docker/config.json/Documents/Cookies/Passwords/Accounts/credentials/secrets/.password*/.token*/.auth*/keychain*/Preferences/*.plist。
decode_file_list 加 SH 第 76-88 行的两层校验:
解码后含 \0 → 拒绝;
任意行不以 / 开头 → 拒绝。
stop_launch_services 的 bundle_id 校验从"任意 alnum.-"收紧到 SH 第 110 行的 reverse-DNS 正则:必须至少含一个 dot,首字符字母数字,段不能空、不能以 - 起头。
refresh_launch_services_after_uninstall 加超时 + system 域 + fallback:
run_with_timeout 10/15 包裹;
主路径 -domain local -domain user -domain system;
124 也算成功;失败回退到去掉 system 的轻量重建。
remove_file_list trash batch 成功后补 log_operation TRASHED(SH 第 277 行)。
完整翻译 SH 第 302-993 行

新增结构 / 函数:

AppDetail:对齐 SH app_details 12 字段管道串。
AppUninstallOutcome:单 app 卸载结果(可序列化给前端)。
BatchUninstallSummary:整批结果(SH 端 print_summary_block 的数据视图)。
collect_app_details:SH 第 348-440 行预扫描——bundle_id/exec_name 解析、running 检测、Caskroom symlink 快速路径 + get_brew_cask_name fallback、sudo 需求(parent 不可写 / owner=root / owner!=current_user / 有 system_files 或 diag_system)、find_app_files + find_app_system_files + get_diagnostic_report_paths_for_app 收集、calculate_total_size 汇总。
uninstall_one_app:SH 第 558-822 行——
stop_launch_services + unregister_app_bundle + remove_login_item + force_kill_app;
brew cask 三态分支(NotInstalled 才走 mole_delete 兜底,Installed/Unknown 报告不同建议);
sudo + symlink:解析目标,/System/* /usr/bin/* /usr/lib/* /bin/* /sbin/* /private/etc/* 拒删;
sudo + 普通文件:走 safe_sudo_remove_ex 拿 i32 退出码,失败时 diagnose_removal_failure 给 reason+suggestion;
非 sudo:mole_delete,失败时区分 parent 不可写 / 其它原因;
容器 stub(com.apple.containermanagerd.metadata.plist)跳过 leftover 计入;
du -skcP 计算 leftover 总大小,从 freed 中扣除;
defaults read 探测后 defaults delete $bundle_id;
ByHost 偏好清理(~/Library/Preferences/ByHost/$bundle_id.*.plist);
/Library/SystemExtensions 检测 → 标 has_system_extension。
batch_uninstall_applications:
收集 details → 计算 total size → 提前 ensure_sudo_session(仅当确实要 sudo);
设 MOLE_UNINSTALL_MODE=1;
逐个 app 跑 uninstall_one_app,聚合 outcomes/warnings;
状态机(success/warn/info)+ 标题 Uninstall complete/incomplete/dry run complete;
收尾:brew_autoremove_silent + remove_apps_from_dock + refresh_launch_services_after_uninstall;
stop_sudo_session + 还原 MOLE_UNINSTALL_MODE。
编译验证
✓ uninstall/optimize/manage 模块完全干净 (其它先存的报错与本次改动无关)
✓ test 编译干净
新增的单测覆盖了:

has_sensitive_data 的命中/不命中
is_valid_reverse_dns_bundle_id 的边界
protected_symlink_target 的系统路径识别
decode_file_list 拒绝相对路径、接受绝对路径
_extract_cask_token_from_path 大小写/起头字符校验
CaskInstallState::is_installed 语义
timeout_for_app size scaling
不变的 GUI 适配点
没翻译 spinner / read_key / 转义序列(纯 TUI),前端用 BatchUninstallSummary 自行渲染。
没翻译 SH 的"按 app 进度推送"——前端可以通过 Tauri event 订阅,或接在 uninstall_one_app 的 callback 上做(当前未加 callback,符合"先翻译,不加功能"的约束)。如果需要进度反馈可以后续再加。
