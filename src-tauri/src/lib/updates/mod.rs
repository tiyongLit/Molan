//! 更新（Updates）功能核心层。
//!
//! 行为权威：Burrow `macos/Sources/{UpdateSources,UpdateCheck,OSUpdateGate,BrewProgress,UpdatesView}.swift`。
//! 与 Mole 无对齐关系（Mole CLI 没有应用更新检查）；唯一复用 Mole 的是应用清单（`mole_list_apps`）。
//!
//! 分层约定（与 lib/uninstall 一致）：本目录只放纯逻辑 + 外部命令执行，
//! Tauri command 薄层在 `controllers/updates.rs`；事件名/payload 在 `crate::events`。

pub mod appcast;
pub mod brew;
pub mod detect;
pub mod itunes;
pub mod version;
