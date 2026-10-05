//! 第三方更新执行引擎（P0：Sparkle 2 + zip 全量包）。
//!
//! 设计蓝图：`docs/更新执行引擎设计方案.md`。
//! 分层：本目录只放纯逻辑与受控的系统调用；Tauri command 薄层在
//! `controllers/updates.rs`；事件常量与 payload 在 `crate::events`。
//!
//! 安全铁律（设计 §4）：任何一道验证门失败 → 丢弃暂存、不触碰系统文件、
//! 报告失败；绝不降级安装。
//!
//! 里程碑进度：
//! - A ✅：feed 解析与选择、下载防护、EdDSA 验签
//! - B ✅：codesign FFI（Apple 锚定 + 身份一致性）
//! - C ✅：staging / archive / relaunch / installer / session（替换回滚全流程）
//! - D：controllers 接线 + 事件 + 前端行内状态机

pub mod archive;
pub mod download;
pub mod feed;
pub mod identity;
pub mod installer;
pub mod relaunch;
pub mod session;
pub mod staging;
pub mod verify;
