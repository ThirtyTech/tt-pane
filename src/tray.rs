use anyhow::Result;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

pub const SETTINGS: &str = "tt-pane.settings";
pub const QUIT: &str = "tt-pane.quit";
#[cfg(windows)]
pub const UPDATE: &str = "tt-pane.update";

pub struct Tray {
    pub icon: TrayIcon,
    #[cfg(windows)]
    pub update: MenuItem,
}

pub fn create() -> Result<Tray> {
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
    let builder = builder.with_icon_templated(Icon::from_rgba(
        include_bytes!("../assets/tray-template.rgba").to_vec(),
        32,
        32,
    )?);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(Icon::from_rgba(
        include_bytes!("../assets/tray.rgba").to_vec(),
        32,
        32,
    )?);
    Ok(Tray {
        icon: builder.build()?,
        #[cfg(windows)]
        update,
    })
}
