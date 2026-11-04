#![forbid(unsafe_code)]

pub mod autostart;
#[cfg(any(windows, target_os = "linux"))]
pub mod centered;
pub mod config;
#[cfg(target_os = "linux")]
pub mod gnome;
pub mod positioning;
pub use tt_pane_platform as platform;
pub use tt_pane_platform::geometry;
pub mod shortcuts;
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
pub mod ui;
#[cfg(windows)]
pub mod update;
