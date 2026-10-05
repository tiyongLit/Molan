//! 验证链子模块。
//!
//! 设计 §4 的五道门在此目录逐步落地：
//! - 门 1 来源验证（EdDSA）→ [`eddsa`] ✅（里程碑 A）
//! - 门 4 落地验证（SecStaticCode + Apple 锚定）→ [`codesign`] ✅（里程碑 B）
//! - 门 5 身份一致性 → `engine/identity.rs` ✅（里程碑 B）

pub mod codesign;
pub mod eddsa;
