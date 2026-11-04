use crate::{
    FocusVisibility,
    geometry::{Action, Rect},
};
use anyhow::{Result, bail};

mod gnome;
mod x11;

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

const FOCUS_UNSUPPORTED: &str =
    "Directional window focus on Linux currently supports GNOME on Wayland only";

pub fn foreground_handle() -> Option<u64> {
    gnome_session().then(gnome::foreground_handle).flatten()
}

pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    if !gnome_session() {
        return Ok(None);
    }
    gnome::window_snapshot(handle)
}

pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    if !gnome_session() {
        bail!("Centered window groups on Linux currently support GNOME on Wayland only");
    }
    gnome::move_snapshot(expected, target)
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    if !gnome_session() {
        bail!(FOCUS_UNSUPPORTED);
    }
    gnome::focus_snapshot(expected)
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
