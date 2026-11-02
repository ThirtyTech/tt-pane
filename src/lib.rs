#![forbid(unsafe_code)]

pub mod config;
pub mod positioning;
pub use stile_platform as platform;
pub use stile_platform::geometry;
pub mod shortcuts;
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
pub mod ui;
