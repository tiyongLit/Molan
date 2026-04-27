//! Maven repository cleanup — 严格对齐 lib/clean/maven.sh
//!
//! 关键点(SH 第 8-15 行):
//!   - 路径 `~/.m2/repository` 默认在白名单里,需要用户从白名单移除才会清
//!   - 清理目标是 `repository/*`(子目录)而不是 `repository` 本身,避免 m2 配置被一并删掉
//!   - 走 safe_clean 把每个 children 走一遍 should_protect_path / whitelist 检查

use std::path::Path;

use crate::core::base::home_dir;
use crate::core::file_ops::safe_clean;

/// 清理 Maven 本地仓库(`~/.m2/repository/*`)。
/// 与 SH 一致:仅当目录存在时执行,删除子目录而非根目录。
/// 返回 `(total_size_kb, total_count)`。
pub fn clean_maven_repository() -> (u64, u64) {
    let maven_repo = format!("{}/.m2/repository", home_dir());
    if !Path::new(&maven_repo).is_dir() {
        return (0, 0);
    }
    let glob = format!("{maven_repo}/*");
    safe_clean(&[&glob], "Maven local repository")
}
