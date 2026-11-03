//! Start tt-pane when the user signs in, using each platform's per-user mechanism:
//! the `HKCU\…\Run` registry value on Windows, a LaunchAgent on macOS and an XDG
//! autostart entry on Linux. None of them needs administrator rights.

use anyhow::{Context, Result};
#[cfg(not(windows))]
use std::path::{Path, PathBuf};

/// Register or remove the sign-in entry so it matches `enabled`. Registering
/// rewrites the entry with the running executable, so a moved app stays current.
pub fn apply(enabled: bool) -> Result<()> {
    let exe = launch_target()?;
    if enabled {
        enable(&exe).context("Could not turn on launch at login")
    } else {
        disable().context("Could not turn off launch at login")
    }
}

/// The program that sign-in should start.
fn launch_target() -> Result<std::path::PathBuf> {
    // An AppImage runs from a temporary mount; start the image itself.
    #[cfg(target_os = "linux")]
    if let Some(image) = std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
        return Ok(image.into());
    }
    let exe = std::env::current_exe().context("Locate the tt-pane executable")?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "tt-pane";

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("Run reg.exe")
}

#[cfg(windows)]
fn enable(exe: &std::path::Path) -> Result<()> {
    let data = format!("\"{}\"", exe.display());
    let out = reg(&[
        "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &data, "/f",
    ])?;
    anyhow::ensure!(
        out.status.success(),
        "reg add failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

#[cfg(windows)]
fn disable() -> Result<()> {
    if !reg(&["query", RUN_KEY, "/v", RUN_VALUE])?.status.success() {
        return Ok(());
    }
    let out = reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])?;
    anyhow::ensure!(
        out.status.success(),
        "reg delete failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

#[cfg(not(windows))]
fn entry_path() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("Could not locate the home directory")?;
    #[cfg(target_os = "macos")]
    return Ok(dirs
        .home_dir()
        .join("Library/LaunchAgents/net.thirtytech.tt-pane.plist"));
    #[cfg(not(target_os = "macos"))]
    Ok(dirs.config_dir().join("autostart/tt-pane.desktop"))
}

#[cfg(not(windows))]
fn enable(exe: &Path) -> Result<()> {
    let path = entry_path()?;
    let contents = entry(exe);
    if std::fs::read_to_string(&path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    let parent = path.parent().context("Autostart entry has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut file, contents.as_bytes())?;
    file.persist(&path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(not(windows))]
fn disable() -> Result<()> {
    match std::fs::remove_file(entry_path()?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// A user LaunchAgent that starts tt-pane once per GUI login. Without
/// `KeepAlive`, quitting from the menu bar keeps it closed until next login.
#[cfg(target_os = "macos")]
fn entry(exe: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>net.thirtytech.tt-pane</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// An XDG autostart entry, honored by GNOME, KDE and most desktop sessions.
#[cfg(all(unix, not(target_os = "macos")))]
fn entry(exe: &Path) -> String {
    // Desktop Entry Exec quoting: wrap in quotes, escape `"`, `` ` ``, `$` and `\`.
    let mut quoted = String::from("\"");
    for c in exe.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        if c == '%' {
            quoted.push('%');
        }
        quoted.push(c);
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=tt-pane\nComment=Keyboard-first window positioning\n\
         Exec={quoted}\nIcon=tt-pane\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn entry_quotes_the_executable() {
        let text = entry(Path::new("/opt/My Apps/tt&pane"));
        #[cfg(target_os = "macos")]
        assert!(text.contains("<string>/opt/My Apps/tt&amp;pane</string>"));
        #[cfg(not(target_os = "macos"))]
        assert!(text.contains("Exec=\"/opt/My Apps/tt&pane\"\n"));
        let odd = entry(Path::new("/a/$b\"c%d"));
        #[cfg(not(target_os = "macos"))]
        assert!(odd.contains(r#"Exec="/a/\$b\"c%%d""#));
        let _ = odd;
    }
}
