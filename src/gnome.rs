//! GNOME Wayland integration: install the bundled Shell extension, which moves
//! windows for `tt-pane apply`, and register tt-pane's shortcuts as GNOME custom
//! keyboard shortcuts that run `tt-pane apply <action>`.
//!
//! Only entries tt-pane owns (paths starting with `tt-pane-`) are written or
//! removed; the user's other custom shortcuts are preserved.

use crate::{config::Config, geometry::Action, shortcuts};
use anyhow::{Context, Result, ensure};
use global_hotkey::hotkey::Modifiers;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub const EXTENSION_UUID: &str = "tt-pane@thirtytech.net";
const EXTENSION_FILES: [(&str, &str); 2] = [
    (
        "metadata.json",
        include_str!("../packaging/gnome/tt-pane@thirtytech.net/metadata.json"),
    ),
    (
        "extension.js",
        include_str!("../packaging/gnome/tt-pane@thirtytech.net/extension.js"),
    ),
];

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM_DIR: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const OWNED_PREFIX: &str = "tt-pane-";

/// What `setup` changed, for a status line the user can act on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Setup {
    /// GNOME only loads a newly installed or updated extension at the next login.
    pub needs_relogin: bool,
    pub shortcuts: usize,
    /// Actions GNOME Wayland can't run yet, left unregistered.
    pub skipped: Vec<Action>,
}

impl Setup {
    pub fn summary(&self) -> String {
        let mut text = format!("Registered {} GNOME shortcuts.", self.shortcuts);
        if !self.skipped.is_empty() {
            text.push_str(&format!(
                " Not yet available on GNOME Wayland: {}.",
                self.skipped
                    .iter()
                    .map(|a| a.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.needs_relogin {
            text.push_str(" Log out and back in once so GNOME loads the tt-pane extension.");
        }
        text
    }
}

/// Install or update the extension and replace tt-pane's GNOME shortcuts.
pub fn setup(config: &Config, executable: &Path, config_path: &Path) -> Result<Setup> {
    let needs_relogin = install_extension()?;
    let (shortcuts, skipped) = sync_shortcuts(config, executable, config_path)?;
    Ok(Setup {
        needs_relogin,
        shortcuts,
        skipped,
    })
}

fn extensions_dir() -> Result<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => directories::BaseDirs::new()
            .context("Could not locate the home directory")?
            .home_dir()
            .join(".local/share"),
    };
    Ok(data.join("gnome-shell/extensions").join(EXTENSION_UUID))
}

/// Returns true when GNOME must reload the extension (installed, updated or newly enabled).
fn install_extension() -> Result<bool> {
    let dir = extensions_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("Create {}", dir.display()))?;
    let mut changed = false;
    for (name, contents) in EXTENSION_FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(contents) {
            std::fs::write(&path, contents).with_context(|| format!("Write {}", path.display()))?;
            changed = true;
        }
    }
    let enabled = parse_strv(&gsettings(&[
        "get",
        "org.gnome.shell",
        "enabled-extensions",
    ])?)?;
    if !enabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        let mut next = enabled;
        next.push(EXTENSION_UUID.to_owned());
        gsettings(&[
            "set",
            "org.gnome.shell",
            "enabled-extensions",
            &format_strv(&next),
        ])?;
        changed = true;
    }
    // Clear a stale "disabled" entry so the new enable sticks.
    let disabled = parse_strv(&gsettings(&[
        "get",
        "org.gnome.shell",
        "disabled-extensions",
    ])?)?;
    if disabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        let next: Vec<_> = disabled
            .into_iter()
            .filter(|u| u != EXTENSION_UUID)
            .collect();
        gsettings(&[
            "set",
            "org.gnome.shell",
            "disabled-extensions",
            &format_strv(&next),
        ])?;
        changed = true;
    }
    // Shells that already know the extension pick up an update or enable without a
    // login only if it is reloaded; a running extension keeps the old code otherwise.
    Ok(changed || !extension_running())
}

fn extension_running() -> bool {
    Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            "net.thirtytech.TtPane",
        ])
        .output()
        .is_ok_and(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("true")
        })
}

/// Actions the GNOME adapter supports. Paired resizing isn't implemented on
/// Linux yet, and Ctrl+Alt+Up/Down would shadow GNOME's workspace switching
/// for nothing.
fn supported(action: Action) -> bool {
    !action.adjusts_pair()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: String,
    name: String,
    command: String,
    binding: String,
}

fn accelerator(mods: Modifiers, key: &str) -> String {
    let mut text = String::new();
    for (flag, name) in [
        (Modifiers::CONTROL, "<Control>"),
        (Modifiers::ALT, "<Alt>"),
        (Modifiers::SHIFT, "<Shift>"),
        (Modifiers::META, "<Super>"),
    ] {
        if mods.contains(flag) {
            text.push_str(name);
        }
    }
    text + key
}

fn entries(
    config: &Config,
    executable: &Path,
    config_path: &Path,
) -> Result<(Vec<Entry>, Vec<Action>)> {
    // GNOME parses the command like a shell and expands desktop-entry % codes.
    let quote = |path: &Path, what: &str| -> Result<String> {
        let text = path.to_string_lossy();
        ensure!(
            !text.contains(['\n', '\r', '"', '$', '`', '\\', '%']),
            "{what} path cannot safely be used in a GNOME shortcut: {text}"
        );
        Ok(format!("\"{text}\""))
    };
    let exe = quote(executable, "Executable")?;
    let config_arg = quote(config_path, "Configuration")?;
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for (key, action) in shortcuts::configured_hotkeys(config)? {
        if !supported(action) {
            if !skipped.contains(&action) {
                skipped.push(action);
            }
            continue;
        }
        let index = entries
            .iter()
            .filter(|e: &&Entry| e.command.ends_with(&format!(" apply {}", action.name())))
            .count();
        entries.push(Entry {
            path: format!("{CUSTOM_DIR}{OWNED_PREFIX}{}-{index}/", action.name()),
            name: format!("tt-pane: {}", action.label()),
            command: format!("{exe} --config {config_arg} apply {}", action.name()),
            binding: accelerator(key.mods, &shortcuts::keysym(key.key)?),
        });
    }
    Ok((entries, skipped))
}

fn owned(path: &str) -> bool {
    path.strip_prefix(CUSTOM_DIR)
        .is_some_and(|rest| rest.starts_with(OWNED_PREFIX))
}

/// The user's entries first, unchanged and in order, then tt-pane's.
fn merged_list(current: &[String], entries: &[Entry]) -> Vec<String> {
    current
        .iter()
        .filter(|path| !owned(path))
        .cloned()
        .chain(entries.iter().map(|e| e.path.clone()))
        .collect()
}

fn sync_shortcuts(
    config: &Config,
    executable: &Path,
    config_path: &Path,
) -> Result<(usize, Vec<Action>)> {
    let executable = std::path::absolute(executable)?;
    let config_path = std::path::absolute(config_path)?;
    let (entries, skipped) = entries(config, &executable, &config_path)?;
    let current = parse_strv(&gsettings(&["get", MEDIA_KEYS, "custom-keybindings"])?)?;
    // Write complete entries before listing them, so GNOME never binds a half-written one.
    for entry in &entries {
        let schema = format!("{CUSTOM}:{}", entry.path);
        for (key, value) in [
            ("name", &entry.name),
            ("command", &entry.command),
            ("binding", &entry.binding),
        ] {
            gsettings(&["set", &schema, key, &gvariant_string(value)])?;
        }
    }
    let next = merged_list(&current, &entries);
    if next != current {
        gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &format_strv(&next)])?;
    }
    for stale in current.iter().filter(|p| owned(p) && !next.contains(p)) {
        gsettings(&["reset-recursively", &format!("{CUSTOM}:{stale}")])?;
    }
    Ok((entries.len(), skipped))
}

fn gsettings(args: &[&str]) -> Result<String> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .context("Run gsettings (is GNOME's glib2 installed?)")?;
    ensure!(
        output.status.success(),
        "gsettings {} failed: {}",
        args.first().unwrap_or(&""),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn gvariant_string(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn format_strv(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| gvariant_string(i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Parse gsettings' printed string array, e.g. `['a', 'b']` or `@as []`.
fn parse_strv(text: &str) -> Result<Vec<String>> {
    let text = text.trim().trim_start_matches("@as").trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .with_context(|| format!("Unexpected gsettings array: {text}"))?;
    let mut items = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        let Some(quote) = chars.next() else { break };
        ensure!(
            quote == '\'' || quote == '"',
            "Unexpected gsettings array: {text}"
        );
        let mut item = String::new();
        loop {
            match chars.next() {
                Some('\\') => item.extend(chars.next()),
                Some(c) if c == quote => break,
                Some(c) => item.push(c),
                None => anyhow::bail!("Unterminated string in gsettings array: {text}"),
            }
        }
        items.push(item);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gsettings_string_arrays() {
        assert_eq!(parse_strv("@as []").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_strv("['/a/custom0/', '/a/custom1/']").unwrap(),
            vec!["/a/custom0/", "/a/custom1/"]
        );
        assert_eq!(parse_strv(r"['it\'s']").unwrap(), vec!["it's"]);
        let items = vec!["x".to_owned(), "it's".to_owned()];
        assert_eq!(parse_strv(&format_strv(&items)).unwrap(), items);
    }

    #[test]
    fn default_shortcuts_become_gnome_custom_keybindings() {
        let (entries, skipped) = entries(
            &Config::default(),
            Path::new("/usr/bin/tt-pane"),
            Path::new("/home/u/.config/tt-pane/config.toml"),
        )
        .unwrap();
        let left = entries
            .iter()
            .find(|e| e.command.ends_with(" apply left"))
            .unwrap();
        assert_eq!(left.binding, "<Control><Alt>1");
        assert_eq!(
            left.command,
            "\"/usr/bin/tt-pane\" --config \"/home/u/.config/tt-pane/config.toml\" apply left"
        );
        assert_eq!(left.path, format!("{CUSTOM_DIR}tt-pane-left-0/"));
        // Expand width also answers Shift+=, as a second entry.
        let expand: Vec<_> = entries
            .iter()
            .filter(|e| e.command.ends_with(" apply expand-width"))
            .map(|e| {
                (
                    e.path.rsplit('/').nth(1).unwrap().to_owned(),
                    e.binding.clone(),
                )
            })
            .collect();
        assert_eq!(
            expand,
            [
                (
                    "tt-pane-expand-width-0".into(),
                    "<Control><Alt>equal".into()
                ),
                (
                    "tt-pane-expand-width-1".into(),
                    "<Control><Alt><Shift>equal".into()
                ),
            ]
        );
        assert!(entries.iter().all(|e| !e.binding.contains("Up")));
        let right = entries
            .iter()
            .find(|e| e.command.ends_with(" apply focus-right"))
            .unwrap();
        assert_eq!(right.binding, "<Control><Alt>l");
        assert_eq!(skipped, [Action::ResizeUp, Action::ResizeDown]);
        let paths: std::collections::HashSet<_> = entries.iter().map(|e| &e.path).collect();
        assert_eq!(paths.len(), entries.len(), "paths must be unique");
    }

    #[test]
    fn keeps_user_shortcuts_and_replaces_only_owned_ones() {
        let current = vec![
            format!("{CUSTOM_DIR}custom0/"),
            format!("{CUSTOM_DIR}tt-pane-left-0/"),
            format!("{CUSTOM_DIR}custom1/"),
            format!("{CUSTOM_DIR}tt-pane-old-0/"),
        ];
        let entry = Entry {
            path: format!("{CUSTOM_DIR}tt-pane-left-0/"),
            name: String::new(),
            command: String::new(),
            binding: String::new(),
        };
        assert_eq!(
            merged_list(&current, &[entry]),
            vec![
                format!("{CUSTOM_DIR}custom0/"),
                format!("{CUSTOM_DIR}custom1/"),
                format!("{CUSTOM_DIR}tt-pane-left-0/"),
            ]
        );
    }

    #[test]
    fn rejects_paths_gnome_would_reinterpret() {
        for bad in [
            "/opt/100%/tt-pane",
            "/opt/$HOME/tt-pane",
            "/opt/\"x\"/tt-pane",
        ] {
            assert!(entries(&Config::default(), Path::new(bad), Path::new("/c.toml")).is_err());
        }
    }
}
