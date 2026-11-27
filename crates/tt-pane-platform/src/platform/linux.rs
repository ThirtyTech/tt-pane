use crate::{
    FocusVisibility,
    geometry::{Action, Rect},
};
use anyhow::{Result, bail};

mod dock;
mod gnome;
mod x11;

pub use dock::{
    DockLayout, DockRequest, activate_handle, cursor_position, dock_layout, dock_monitor,
    dock_parked, dock_picking, dock_running, docked_window, hide_dock, is_minimized, lift_window,
    moving_window, park_window, pending_dock_pick, pick_dock, place_window, set_dock_picking,
    set_docked, show_dock, stow_window, take_dock_clicks, take_dock_moves,
};
pub use gnome::{WindowSnapshot, is_gnome};

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    if super::is_wayland() {
        if gnome::is_gnome() {
            return gnome::apply(resolve);
        }
        bail!(
            "This Wayland compositor has no tt-pane adapter yet. GNOME is supported on Wayland; other desktops work in an X11 session. XWayland cannot control native Wayland windows."
        );
    }
    x11::apply(resolve)
}

/// Window inspection and focus go through the GNOME Shell extension; other
/// Linux sessions have no such adapter yet.
fn gnome_session() -> bool {
    super::is_gnome_wayland()
}

/// X11 sessions inspect and move windows through EWMH, for tiling.
pub fn x11_session() -> bool {
    !super::is_wayland() && std::env::var_os("DISPLAY").is_some()
}

const FOCUS_UNSUPPORTED: &str =
    "Directional window focus on Linux currently supports GNOME on Wayland only";

const WINDOWS_UNSUPPORTED: &str =
    "Window inspection on Linux supports GNOME on Wayland and X11 sessions only";

pub fn foreground_handle() -> Option<u64> {
    if gnome_session() {
        gnome::foreground_handle()
    } else if x11_session() {
        x11::foreground_handle()
    } else {
        None
    }
}

pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    if gnome_session() {
        gnome::window_snapshot(handle)
    } else if x11_session() {
        x11::window_snapshot(handle)
    } else {
        Ok(None)
    }
}

pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    if gnome_session() {
        gnome::move_snapshot(expected, target)
    } else if x11_session() {
        x11::move_snapshot(expected, target)
    } else {
        bail!(WINDOWS_UNSUPPORTED)
    }
}

const TILING_UNSUPPORTED: &str =
    "Automatic tiling on Linux supports GNOME on Wayland and X11 sessions only";

pub fn tiling_desktop() -> Result<crate::TilingDesktop> {
    if gnome_session() {
        gnome::tiling_desktop()
    } else if x11_session() {
        x11::tiling_desktop()
    } else {
        bail!(TILING_UNSUPPORTED)
    }
}

pub fn move_tiling_window(expected: &crate::TilingWindow, target: Rect) -> Result<Rect> {
    move_snapshot(&expected.snapshot, target)
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    if gnome_session() {
        gnome::focus_snapshot(expected)
    } else if x11_session() {
        x11::focus_snapshot(expected)
    } else {
        bail!(WINDOWS_UNSUPPORTED)
    }
}

pub fn focus_window(
    handle: Option<u64>,
    action: Action,
    visibility: FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<()> {
    if !gnome_session() {
        bail!(FOCUS_UNSUPPORTED);
    }
    gnome::focus_window(handle, action, visibility, group, middle_members)
}

pub fn focus_eligible_handles(handle: u64, visibility: FocusVisibility) -> Result<Vec<u64>> {
    if !gnome_session() {
        bail!(FOCUS_UNSUPPORTED);
    }
    gnome::focus_eligible_handles(handle, visibility)
}
