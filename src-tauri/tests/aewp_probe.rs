//! AEWP 提权身份探针：实测 AuthorizationExecuteWithPrivileges 链路下
//! 工具进程的真实 uid/gid 与文件属主，定位 DNS 两项 root 命令失败的根因；
//! 并实测三个 killall 替代候选（pkill / pgrep+kill / launchctl）。
//!
//! 背景：Optimize 的 DNS 两项（dscacheutil + killall -HUP mDNSResponder）经
//! AEWP → /bin/sh -p -c 链路执行时，killall 报 "No matching processes
//! belonging to you were found"（权限不足特征）；而原版 mole 走系统 sudo
//! 票据一次成功。本探针直接观测 AEWP 交付的执行身份，一锤定音。
//!
//! 运行（会弹原生认证面板，需人工输一次密码）：
//!   cargo test --test aewp_probe -- --nocapture --test-threads=1
//!
//! 探测矩阵：
//!   [1] ensure_admin_session       建立 auth_ref（认证层）
//!   [2] /usr/bin/true              基线：AEWP 链路是否通
//!   [3] sh -c "id"                 核心：工具进程真实身份（uid/euid/gid/groups）
//!   [4] sh -c "touch+stat"         行为级：AEWP 下创建文件的属主
//!   [5] sh -p -c "id"              对照：显式 -p 是否改变身份
//!   [6] killall -HUP mDNSResponder 复现：DNS 失败现场
//!   [7] dscacheutil -flushcache    对照：普通用户也能成功的命令
//!   [8] killall -l                 对照：killall 本身可执行（无需 root）
//!   [9] pkill -HUP mDNSResponder   修复候选 A：白名单直连（无 sh 包装）
//!   [10] kill -HUP $(pgrep …)      修复候选 B：按 pid 直发（含 pgrep 枚举诊断）
//!   [11] launchctl kill 1 system/… 修复候选 C：直达 launchd（不经进程枚举）

use mole_lib::core::sudo::{ensure_admin_session, is_admin_authorized, sudo_output};

fn show(label: &str, out: &std::process::Output) {
    println!("--- {label} ---");
    println!("  exit = {:?}", out.status.code());
    let so = String::from_utf8_lossy(&out.stdout);
    let se = String::from_utf8_lossy(&out.stderr);
    if !so.trim().is_empty() {
        for line in so.trim_end().lines() {
            println!("  stdout| {line}");
        }
    }
    if !se.trim().is_empty() {
        for line in se.trim_end().lines() {
            println!("  stderr| {line}");
        }
    }
}

#[test]
fn probe_aewp_identity() {
    println!();
    println!("================================================================");
    println!("AEWP 提权身份探针（即将弹出原生认证面板，请输一次密码）");
    println!("================================================================");

    println!("\n[1] 建立管理员会话（认证层）...");
    let ok = ensure_admin_session();
    println!("    ensure_admin_session => {ok}");
    println!("    is_admin_authorized  => {}", is_admin_authorized());
    if !ok {
        println!("    认证被取消/失败，探针终止。");
        return;
    }

    println!("\n[2] 基线：/usr/bin/true（白名单内，无副作用）");
    show("true", &sudo_output(&["/usr/bin/true"]));

    println!("\n[3] 核心：AEWP 链路下工具进程的真实身份");
    show("sh -c id", &sudo_output(&["/bin/sh", "-c", "id"]));
    show(
        "sh -c id -u/ru/G",
        &sudo_output(&["/bin/sh", "-c", "id -u; id -ru; id -G"]),
    );

    println!("\n[4] 行为级：AEWP 下创建文件的属主（root 还是你？）");
    show(
        "sh -c touch+stat",
        &sudo_output(&[
            "/bin/sh",
            "-c",
            "rm -f /tmp/aewp_probe_owner; touch /tmp/aewp_probe_owner; stat -f 'owner=%Su group=%Sg perms=%Sp' /tmp/aewp_probe_owner",
        ]),
    );

    println!("\n[5] 对照：/bin/sh -p（显式 privileged 模式）下的身份");
    show(
        "sh -p -c id",
        &sudo_output(&["/bin/sh", "-p", "-c", "id; id -u; id -ru"]),
    );

    println!("\n[6] 复现：killall -HUP mDNSResponder（DNS 失败现场）");
    show(
        "killall -HUP mDNSResponder",
        &sudo_output(&["/usr/bin/killall", "-HUP", "mDNSResponder"]),
    );

    println!("\n[7] 对照：dscacheutil -flushcache（普通用户身份也能 rc=0）");
    show(
        "dscacheutil -flushcache",
        &sudo_output(&["/usr/bin/dscacheutil", "-flushcache"]),
    );

    println!("\n[8] 对照：killall -l（列信号名，无需 root）");
    let out = sudo_output(&["/usr/bin/killall", "-l"]);
    println!("--- killall -l ---");
    println!("  exit = {:?}", out.status.code());
    let so = String::from_utf8_lossy(&out.stdout);
    println!(
        "  stdout(前120字符) = {}",
        so.chars().take(120).collect::<String>().replace('\n', " ")
    );

    println!("\n[9] 修复候选 A：pkill -HUP mDNSResponder（白名单直连，无 sh 包装）");
    println!("    说明：pkill 按模式匹配进程找到即发信号，不走 killall 的归属过滤");
    show(
        "pkill -HUP mDNSResponder",
        &sudo_output(&["/usr/bin/pkill", "-HUP", "mDNSResponder"]),
    );

    println!("\n[10] 修复候选 B：kill -HUP $(pgrep mDNSResponder)（按 pid 直发）");
    println!("  [10a] 先验证 pgrep 能否在 AEWP 下枚举进程（是否受 real-uid 过滤）");
    show(
        "sh -c pgrep -l mDNSResponder",
        &sudo_output(&["/bin/sh", "-c", "pgrep -l mDNSResponder"]),
    );
    println!("  [10b] 按 pid 发 HUP（euid=0 内核层面应放行）");
    show(
        "sh -c kill -HUP $(pgrep mDNSResponder)",
        &sudo_output(&["/bin/sh", "-c", "kill -HUP $(pgrep mDNSResponder)"]),
    );

    println!("\n[11] 修复候选 C：launchctl kill 1 system/com.apple.mDNSResponder（直达 launchd）");
    println!("    说明：SIGHUP=1；launchd 直接对服务发信号，不经进程枚举，语义最正统");
    show(
        "launchctl kill 1 system/com.apple.mDNSResponder",
        &sudo_output(&[
            "/bin/launchctl",
            "kill",
            "1",
            "system/com.apple.mDNSResponder",
        ]),
    );

    println!();
    println!("================================================================");
    println!("判读指南：");
    println!("  [3]/[5] uid=0(root)         → AEWP 交付了 root，问题在别处");
    println!("  [3]/[5] uid=501(你的用户名)  → AEWP 未交付 root（提权失效实锤）");
    println!("  [3]/[5] 同时显示 euid=0      → 半 root 状态（bash 降权理论成立）");
    println!("  [4] owner=root              → root 实锤");
    println!("  [4] owner=你的用户名         → 未交付 root");
    println!("  [6] exit=0                  → killall 有 root 权限");
    println!("  [6] 'No matching processes belonging to you' → 无 root（复现失败）");
    println!("  [9]/[10b]/[11] exit=0       → 该候选可用（信号已送达，euid=0 足够）");
    println!("  [9]/[10b]/[11] exit=1       → 该候选同样被挡（需换提权机制）");
    println!("  [10a] 列出 pid              → pgrep 不受 real-uid 过滤（候选 B 可行）");
    println!("  [10a] 无输出                → pgrep 也被过滤，候选 B 不可行");
    println!("================================================================");
    println!();
}
