//! 版本比较与系统兼容门。
//! 语义基线见 `controllers/updates.md` §4.4 核心函数规格。
//!
//! ⚠️ 禁止复用 `lib/clean/user.rs::version_compare`：那是 sort -V 语义
//! （不剥 v 前缀、非数字段字典序、不等长直接比长度），与本模块语义均不同。

/// 判定 remote 是否比 local 新：
/// trim → 剥离一个前导 v/V → 按 '.' 拆分 → 每段整数（解析失败归 0）→
/// 缺段补 0 逐段比 → 全等返回 false。
pub fn is_version_newer(remote: &str, local: &str) -> bool {
    let r = version_parts(remote);
    let l = version_parts(local);
    for i in 0..r.len().max(l.len()) {
        let rp = r.get(i).copied().unwrap_or(0);
        let lp = l.get(i).copied().unwrap_or(0);
        if rp != lp {
            return rp > lp;
        }
    }
    false
}

fn version_parts(s: &str) -> Vec<i64> {
    let v = s.trim();
    let v = v
        .strip_prefix('v')
        .or_else(|| v.strip_prefix('V'))
        .unwrap_or(v);
    v.split('.')
        .map(|p| p.parse::<i64>().unwrap_or(0))
        .collect()
}

/// 系统兼容门：
/// minimum 为空/空白 → 可安装；否则 running >= minimum（点分逐段，缺段补 0，全等算满足）。
pub fn os_is_installable(minimum_os: Option<&str>, running_os: &str) -> bool {
    let Some(m) = minimum_os.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    os_at_least(os_parse(running_os), os_parse(m))
}

/// 解析系统版本：点分版本，每段取前导数字，解析失败归 0。
fn os_parse(v: &str) -> Vec<i64> {
    v.split('.')
        .map(|p| {
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<i64>().unwrap_or(0)
        })
        .collect()
}

/// 点分逐段比较：a >= b，全等返回 true。
fn os_at_least(a: Vec<i64>, b: Vec<i64>) -> bool {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    true
}
