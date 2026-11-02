use crate::geometry::Rect;
use anyhow::{Result, bail};

mod wayland;
mod x11;

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    if super::is_wayland() {
        if std::env::var_os("SWAYSOCK").is_some() {
            return wayland::sway(resolve);
        }
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return wayland::hyprland(resolve);
        }
        bail!(
            "This Wayland compositor has no sTile adapter yet. Sway and Hyprland are supported; GNOME requires a Shell extension and KDE requires a KWin script. XWayland cannot control native Wayland windows."
        );
    }
    x11::apply(resolve)
}
