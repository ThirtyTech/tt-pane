use anyhow::Result;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

pub const SETTINGS: &str = "stile.settings";
pub const QUIT: &str = "stile.quit";

pub fn create() -> Result<TrayIcon> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id(SETTINGS, "Settings…", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit sTile", true, None),
    ])?;
    let builder = TrayIconBuilder::new()
        .with_id("stile")
        .with_tooltip("sTile — window shortcuts")
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
    Ok(builder.build()?)
}
