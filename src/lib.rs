#![forbid(unsafe_code)]

pub mod autostart;
#[cfg(any(windows, target_os = "linux"))]
pub mod centered;
pub mod config;
#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(all(target_os = "linux", feature = "gui"))]
mod launcher;
pub mod positioning;
#[cfg(windows)]
pub mod tiling;
pub use tt_pane_platform as platform;
pub use tt_pane_platform::geometry;
pub mod shortcuts;
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
pub mod ui;
pub mod update;
