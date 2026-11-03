//! Native integration-test scenarios. Enabled only by the test-support feature.
mod commands;
mod tray_lifecycle;
pub use commands::binary_positions_disposable_window_on_every_monitor;
pub use tray_lifecycle::tray_starts_without_window_and_survives_settings_close;

pub use tray_lifecycle::terminal_ctrl_c_terminates_tray_app;
