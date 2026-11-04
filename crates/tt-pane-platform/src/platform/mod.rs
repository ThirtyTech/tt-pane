use crate::geometry::{Action, Rect};
use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows;

pub fn apply(action: Action) -> Result<()> {
    if action.focuses() {
        return focus_window(None, action);
    }
    anyhow::ensure!(
        action != Action::FillGap && !action.adjusts_pair(),
        "This action requires the positioning coordinator"
    );
    apply_with(None, &mut |_, area, current| {
        Ok(action.target(area, current))
    })
    .map(|_| ())
}

/// Resolve a target after capturing the native window and its work area.
/// The backend keeps targeting that same window even if foreground focus changes.
/// Returns observed bounds on synchronous backends, requested bounds on IPC backends.
pub fn apply_with(
    handle: Option<u64>,
    resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>,
) -> Result<Rect> {
    #[cfg(target_os = "windows")]
    return windows::apply(handle, resolve);
    #[cfg(not(target_os = "windows"))]
    anyhow::ensure!(
        handle.is_none(),
        "Explicit --window targeting currently supports Windows HWNDs only"
    );
    #[cfg(target_os = "macos")]
    return macos::apply(resolve);
    #[cfg(target_os = "linux")]
    return linux::apply(resolve);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    anyhow::bail!("Unsupported operating system")
}

/// Explicit native window targeting for automation. HWNDs are Windows-specific.
pub fn apply_to_window(action: Action, handle: u64) -> Result<()> {
    if action.focuses() {
        return focus_window(Some(handle), action);
    }
    anyhow::ensure!(
        action != Action::FillGap && !action.adjusts_pair(),
        "This action requires the positioning coordinator"
    );
    apply_with(Some(handle), &mut |_, area, current| {
        Ok(action.target(area, current))
    })
    .map(|_| ())
}

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland"))
}

/// GNOME Wayland sessions, which need the bundled Shell extension.
pub fn is_gnome_wayland() -> bool {
    is_wayland() && is_gnome()
}

#[cfg(target_os = "linux")]
use linux::is_gnome;
#[cfg(not(target_os = "linux"))]
fn is_gnome() -> bool {
    false
}

pub fn description() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows · native Win32"
    } else if cfg!(target_os = "macos") {
        "macOS · Accessibility"
    } else if is_wayland() {
        if is_gnome() {
            "Wayland · GNOME Shell extension"
        } else {
            "Wayland · compositor not yet supported"
        }
    } else {
        "Linux · X11 / EWMH"
    }
}

/// Centered groups on Linux: implemented for GNOME on Wayland, inert elsewhere.
#[cfg(target_os = "linux")]
pub use linux::{
    WindowSnapshot, focus_snapshot, foreground_handle, move_snapshot, window_snapshot,
};
#[cfg(target_os = "macos")]
pub use macos::{accessibility_trusted, show_alert};
#[cfg(windows)]
pub use windows::attach_parent_console;
#[cfg(all(windows, feature = "window-icon"))]
pub use windows::set_window_icon;
#[cfg(windows)]
pub use windows::{
    WindowSnapshot, choose_glow_color, focus_snapshot, foreground_handle, move_snapshot,
    show_focus_glow, wait_for_focus_glow, window_snapshot,
};

/// Move the shared boundary of a window and its vertical neighbor.
pub fn resize_pair(handle: Option<u64>, up: bool) -> Result<()> {
    #[cfg(target_os = "windows")]
    return windows::resize_pair(handle, up);
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (handle, up);
        anyhow::bail!("Paired vertical resizing currently supports Windows only")
    }
}

/// Focus a spatial neighbor without changing placement or saved resize state.
pub fn focus_window(handle: Option<u64>, action: Action) -> Result<()> {
    focus_window_with_visibility(handle, action, false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusVisibility {
    BackgroundOnly,
    MoreThanPercent(u32),
    AtLeastHalf,
    IncludeHidden,
    /// Enter covered windows from an edge; exit middle windows to background edges.
    Hold,
}

/// Edge-snapped windows qualify regardless of occlusion. Floating middle windows
/// require at least 50% visible area unless include_obscured is enabled.
pub fn focus_window_with_visibility(
    handle: Option<u64>,
    action: Action,
    include_obscured: bool,
) -> Result<()> {
    focus_window_outside_group(handle, action, include_obscured, &[])
}

/// Group exits skip members on the same monitor; vertical exits prefer leftmost windows.
/// An empty group retains ordinary spatial navigation.
pub fn focus_window_outside_group(
    handle: Option<u64>,
    action: Action,
    include_obscured: bool,
    group: &[u64],
) -> Result<()> {
    let visibility = if include_obscured {
        FocusVisibility::IncludeHidden
    } else {
        FocusVisibility::AtLeastHalf
    };
    focus_window_filtered(handle, action, visibility, group)
}

/// Filter middle windows by visible area; edge snaps remain reachable.
pub fn focus_window_filtered(
    handle: Option<u64>,
    action: Action,
    visibility: FocusVisibility,
    group: &[u64],
) -> Result<()> {
    focus_window_with_members(handle, action, visibility, group, &[])
}

/// Explicit Center members remain middle windows even when grown to an edge.
pub fn focus_window_with_members(
    handle: Option<u64>,
    action: Action,
    visibility: FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<()> {
    anyhow::ensure!(action.focuses(), "Expected a directional focus action");
    #[cfg(target_os = "windows")]
    return windows::focus_window(handle, action, visibility, group, middle_members);
    #[cfg(target_os = "macos")]
    {
        anyhow::ensure!(
            handle.is_none(),
            "Explicit --window targeting currently supports Windows HWNDs only"
        );
        // Centered groups are Windows and GNOME only, so no group or members reach macOS.
        let _ = (group, middle_members);
        macos::focus_window(action, visibility)
    }
    #[cfg(target_os = "linux")]
    return linux::focus_window(handle, action, visibility, group, middle_members);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (handle, action, visibility, group, middle_members);
        anyhow::bail!("Directional window focus currently supports Windows, macOS and GNOME only")
    }
}

/// Eligible destinations in live Z-order, also used for centered-group traversal.
pub fn focus_eligible_handles(handle: u64, visibility: FocusVisibility) -> Result<Vec<u64>> {
    #[cfg(windows)]
    return windows::focus_eligible_handles(handle, visibility);
    #[cfg(target_os = "linux")]
    return linux::focus_eligible_handles(handle, visibility);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (handle, visibility);
        anyhow::bail!("Directional window focus currently supports Windows and GNOME only")
    }
}
