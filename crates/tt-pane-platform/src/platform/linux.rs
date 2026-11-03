use crate::geometry::Rect;
use anyhow::{Result, bail};

mod gnome;
mod wayland;
mod x11;

pub use gnome::is_gnome;

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    if super::is_wayland() {
        if std::env::var_os("SWAYSOCK").is_some() {
            return wayland::sway(resolve);
        }
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return wayland::hyprland(resolve);
        }
        if gnome::is_gnome() {
            return gnome::apply(resolve);
        }
        bail!(
            "This Wayland compositor has no tt-pane adapter yet. Sway, Hyprland and GNOME are supported; KDE requires a KWin script. XWayland cannot control native Wayland windows."
        );
    }
    x11::apply(resolve)
}
