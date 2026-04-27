//! 启动项管理核心层：多源发现 + App 关联 + 操作引擎。
//! 后端对齐 Launchdeck（discovery/model/actions），前端对齐 Lemon Cleaner（App 分组 UI）。
//! spawn 是 seam：纯函数（model/filter/app_assoc/login_items）与进程执行（discovery/actions）分离。

pub mod actions;
pub mod app_assoc;
pub mod discovery;
pub mod filter;
pub mod login_items;
pub mod model;
