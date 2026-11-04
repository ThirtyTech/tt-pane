//! Linux desktop entry and icon for tt-pane's windows.
//!
//! Wayland has no per-window icon: GNOME and KDE find a window's icon through
//! the desktop entry named after its app id. `install.sh` writes one, but a
//! binary run from anywhere else (an archive, an AppImage, `cargo run`) would
//! show a generic icon, so tt-pane installs its own when none exists. An entry
//! someone else wrote (install.sh, a package) is never touched.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Matches the `app_id` the settings window opens with.
pub const APP_ID: &str = "dev.ttpane.TtPane";
const ICON: &[u8] = include_bytes!("../assets/tt-pane.png");
/// Marks an entry tt-pane wrote itself, so it may keep it current.
const MARKER: &str = "X-TtPane-Generated=true";

/// Make sure the desktop can show tt-pane's icon for its windows.
pub fn ensure() -> Result<()> {
    let dirs = directories::BaseDirs::new().context("Could not locate the home directory")?;
    let data = dirs.data_dir();
    write_if_changed(&data.join("icons/hicolor/256x256/apps/tt-pane.png"), ICON)
        .context("Could not install the tt-pane icon")?;
    let name = format!("{APP_ID}.desktop");
    let path = data.join("applications").join(&name);
    match std::fs::read_to_string(&path) {
        // A debug build never replaces an entry, so `cargo run` can't hide or
        // repoint a release's launcher.
        Ok(current) if !current.contains(MARKER) || cfg!(debug_assertions) => return Ok(()),
        Err(_)
            if system_dirs()
                .iter()
                .any(|dir| dir.join("applications").join(&name).exists()) =>
        {
            return Ok(());
        }
        _ => {}
    }
    let exe = crate::autostart::launch_target()?;
    // A debug build's launcher would point into target/; keep it out of the
    // app grid while still naming the icon for the window.
    write_if_changed(&path, entry(&exe, cfg!(debug_assertions)).as_bytes())
        .context("Could not install the tt-pane desktop entry")
}

/// `XDG_DATA_DIRS`, where packages and system-wide installs put entries.
fn system_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.split(':')
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn entry(exe: &Path, hidden: bool) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=tt-pane\nComment=Place windows with keyboard shortcuts\n\
         Exec={}\nIcon=tt-pane\nTerminal=false\nCategories=Utility;\nStartupWMClass={APP_ID}\n{}{MARKER}\n",
        crate::autostart::exec_quote(exe),
        if hidden { "NoDisplay=true\n" } else { "" },
    )
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    let parent = path.parent().context("Path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut file, contents)?;
    // Temp files start private; desktop files and icons are normally 0644.
    file.as_file()
        .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_the_icon_and_window_class() {
        let text = entry(Path::new("/opt/tt pane/tt-pane"), false);
        assert!(text.contains("Exec=\"/opt/tt pane/tt-pane\"\n"));
        assert!(text.contains("Icon=tt-pane\n"));
        assert!(text.contains(&format!("StartupWMClass={APP_ID}\n")));
        assert!(text.contains(MARKER));
        assert!(!text.contains("NoDisplay"));
        assert!(entry(Path::new("/x"), true).contains("NoDisplay=true\n"));
    }
}
