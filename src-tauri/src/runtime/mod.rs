//! GUI 运行时接线层 — 托盘、菜单、Dock 生命周期、后台 watcher。
//! 由 `lib.rs` 启动期挂接；命令名/事件名契约与 controllers 保持一致。

pub mod app_menu;
pub mod fda_guide;
pub mod macos_dock_quit;
pub mod residual_watch;
pub mod trash_watch;
pub mod tray;
