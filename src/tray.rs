use crate::config::TrayIconStyle;
use anyhow::Result;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

pub const SETTINGS: &str = "tt-pane.settings";
pub const QUIT: &str = "tt-pane.quit";
#[cfg(windows)]
pub const UPDATE: &str = "tt-pane.update";

const SIZE: u32 = 32;
const COLOR: &[u8] = include_bytes!("../assets/tray.rgba");
/// Black silhouette with alpha; the macOS template image.
const TEMPLATE: &[u8] = include_bytes!("../assets/tray-template.rgba");

pub struct Tray {
    pub icon: TrayIcon,
    #[cfg(windows)]
    pub update: MenuItem,
}

pub fn create(style: TrayIconStyle) -> Result<Tray> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id(SETTINGS, "Settings…", true, None))?;
    #[cfg(windows)]
    let update = {
        let item = MenuItem::with_id(UPDATE, "Check for updates…", true, None);
        menu.append(&item)?;
        item
    };
    menu.append_items(&[
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit tt-pane", true, None),
    ])?;
    let builder = TrayIconBuilder::new()
        .with_id("tt-pane")
        .with_tooltip("tt-pane — window shortcuts")
        .with_menu(Box::new(menu));
    #[cfg(target_os = "macos")]
    let builder = if is_template(style) {
        builder.with_icon_templated(icon(style)?)
    } else {
        builder.with_icon(icon(style)?)
    };
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(icon(style)?);
    Ok(Tray {
        icon: builder.build()?,
        #[cfg(windows)]
        update,
    })
}

/// Switch a running tray icon to `style`.
pub fn set_style(tray: &TrayIcon, style: TrayIconStyle) -> Result<()> {
    #[cfg(target_os = "macos")]
    if is_template(style) {
        return Ok(tray.set_icon_templated(Some(icon(style)?))?);
    }
    tray.set_icon(Some(icon(style)?))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_template(style: TrayIconStyle) -> bool {
    style == TrayIconStyle::Monochrome
}

fn icon(style: TrayIconStyle) -> Result<Icon> {
    Ok(Icon::from_rgba(pixels(style), SIZE, SIZE)?)
}

fn pixels(style: TrayIconStyle) -> Vec<u8> {
    // Windows has no monochrome option.
    if cfg!(windows) || style == TrayIconStyle::Color {
        return COLOR.to_vec();
    }
    if cfg!(target_os = "macos") {
        // macOS tints a template image itself; only its alpha matters.
        return TEMPLATE.to_vec();
    }
    // Linux trays draw pixmaps as given, so paint the silhouette white for the
    // dark panels most desktops use (GNOME's top bar included).
    TEMPLATE
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| [255, 255, 255, px[3]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_full_size() {
        let len = (SIZE * SIZE * 4) as usize;
        assert_eq!(pixels(TrayIconStyle::Color).len(), len);
        assert_eq!(pixels(TrayIconStyle::Monochrome).len(), len);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_monochrome_is_white_with_template_alpha() {
        let mono = pixels(TrayIconStyle::Monochrome);
        for (px, template) in mono
            .as_chunks::<4>()
            .0
            .iter()
            .zip(TEMPLATE.as_chunks::<4>().0)
        {
            assert_eq!(&px[..3], &[255, 255, 255]);
            assert_eq!(px[3], template[3]);
        }
    }
}
