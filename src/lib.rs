pub mod config;
pub mod geometry;
pub mod platform;
pub mod shortcuts;
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
pub mod ui;
