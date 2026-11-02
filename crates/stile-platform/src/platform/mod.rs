use crate::geometry::Action;
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
    #[cfg(target_os = "windows")]
    return windows::apply(action);
    #[cfg(target_os = "macos")]
    return macos::apply(action);
    #[cfg(target_os = "linux")]
    return linux::apply(action);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    anyhow::bail!("Unsupported operating system")
}

/// Explicit native window targeting for automation. HWNDs are Windows-specific.
pub fn apply_to_window(action: Action, handle: u64) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let handle = usize::try_from(handle)?;
        windows::apply_to(action, ::windows::Win32::Foundation::HWND(handle as *mut _))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (action, handle);
        anyhow::bail!("Explicit --window targeting currently supports Windows HWNDs only")
    }
}

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland"))
}

pub fn description() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows · native Win32"
    } else if cfg!(target_os = "macos") {
        "macOS · Accessibility"
    } else if is_wayland() {
        if std::env::var_os("SWAYSOCK").is_some() {
            "Wayland · Sway IPC"
        } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            "Wayland · Hyprland IPC"
        } else {
            "Wayland · compositor not yet supported"
        }
    } else {
        "Linux · X11 / EWMH"
    }
}

#[cfg(windows)]
pub use windows::attach_parent_console;
#[cfg(all(windows, feature = "window-icon"))]
pub use windows::set_window_icon;
