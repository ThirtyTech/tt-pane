use crate::geometry::Action;
use anyhow::{Context, Result, ensure};
use global_hotkey::hotkey::HotKey;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub action: Action,
    pub shortcut: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub growth: Growth,
    #[serde(default)]
    pub width_resize: WidthResize,
    #[serde(default)]
    pub focus: Focus,
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Growth {
    pub percent: u32,
    pub steps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WidthResize {
    pub percent: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "FocusInput")]
pub struct Focus {
    pub minimum_visible_percent: u32,
    pub glow_enabled: bool,
    pub glow_color: String,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FocusInput {
    minimum_visible_percent: Option<u32>,
    include_obscured: Option<bool>,
    glow_enabled: Option<bool>,
    glow_color: Option<String>,
}

impl From<FocusInput> for Focus {
    fn from(input: FocusInput) -> Self {
        let defaults = Self::default();
        Self {
            minimum_visible_percent: input.minimum_visible_percent.unwrap_or_else(|| {
                if input.include_obscured == Some(false) {
                    50
                } else {
                    10
                }
            }),
            glow_enabled: input.glow_enabled.unwrap_or(defaults.glow_enabled),
            glow_color: input.glow_color.unwrap_or(defaults.glow_color),
        }
    }
}

impl Default for Focus {
    fn default() -> Self {
        Self {
            minimum_visible_percent: 10,
            glow_enabled: true,
            glow_color: "#F2F0E8".into(),
        }
    }
}

impl Focus {
    pub fn glow_rgb(&self) -> Option<u32> {
        let hex = self.glow_color.strip_prefix('#')?;
        (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
    }
}

impl Default for WidthResize {
    fn default() -> Self {
        Self { percent: 25 }
    }
}

impl Default for Growth {
    fn default() -> Self {
        Self {
            percent: 15,
            steps: 2,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let keys = [
            "1",
            "2",
            "8",
            "5",
            "7",
            "9",
            "4",
            "6",
            "3",
            "0",
            "-",
            "=",
            "Shift+8",
            "ArrowUp",
            "ArrowDown",
            "H",
            "J",
            "K",
            "L",
        ];
        Self {
            version: 1,
            growth: Growth::default(),
            width_resize: WidthResize::default(),
            focus: Focus::default(),
            bindings: Action::ALL
                .into_iter()
                .zip(keys)
                .map(|(action, key)| Binding {
                    action,
                    shortcut: format!("Ctrl+Alt+{key}"),
                })
                .collect(),
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let path = directories::ProjectDirs::from("", "", "tt-pane")
            .context("Could not locate the user configuration directory")?
            .config_dir()
            .join("config.toml");
        // Keep existing settings and adjacent cycle state usable after the rebrand.
        let legacy = directories::ProjectDirs::from("", "", "stile")
            .context("Could not locate the user configuration directory")?
            .config_dir()
            .join("config.toml");
        Self::existing_config_path(path, legacy)
    }

    fn existing_config_path(path: PathBuf, legacy: PathBuf) -> Result<PathBuf> {
        if !path.try_exists()? && legacy.try_exists()? {
            return Ok(legacy);
        }
        Ok(path)
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut config: Self =
                    toml::from_str(&text).context("Invalid configuration TOML")?;
                config.validate()?;
                config.add_missing_focus_bindings();
                config.validate()?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("Read {}", path.display())),
        }
    }

    // Introduce new focus defaults without overriding custom or disabled actions.
    // A default already assigned to another action stays unbound in Settings.
    fn add_missing_focus_bindings(&mut self) {
        for mut binding in Self::default()
            .bindings
            .into_iter()
            .filter(|b| b.action.focuses())
        {
            if self.bindings.iter().any(|b| b.action == binding.action) {
                continue;
            }
            let key = binding.shortcut.parse::<HotKey>().unwrap();
            if self.bindings.iter().any(|b| {
                b.shortcut
                    .parse::<HotKey>()
                    .is_ok_and(|k| k.id() == key.id())
            }) {
                binding.shortcut.clear();
            }
            self.bindings.push(binding);
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.focus.minimum_visible_percent <= 99,
            "Focus visibility percentage must be between 0 and 99"
        );
        ensure!(
            self.focus.glow_rgb().is_some(),
            "Focus glow color must be #RRGGBB"
        );
        ensure!(
            (1..=99).contains(&self.width_resize.percent),
            "Width adjustment percentage must be between 1 and 99"
        );
        ensure!(
            (1..=100).contains(&self.growth.percent),
            "Growth percentage must be between 1 and 100"
        );
        ensure!(
            self.growth.steps <= 20,
            "Growth steps must be between 0 and 20 (0 disables growth)"
        );
        ensure!(
            self.version == 1,
            "Unsupported configuration version {}",
            self.version
        );
        let mut actions = HashSet::new();
        let mut keys = HashSet::new();
        for binding in &self.bindings {
            ensure!(
                actions.insert(binding.action),
                "Duplicate action {}",
                binding.action.name()
            );
            if binding.shortcut.is_empty() {
                continue;
            }
            let key: HotKey = binding
                .shortcut
                .parse()
                .with_context(|| format!("Invalid shortcut for {}", binding.action.name()))?;
            ensure!(
                !key.mods.is_empty(),
                "Shortcut {} needs a modifier",
                binding.shortcut
            );
            ensure!(
                keys.insert(key.id()),
                "Duplicate shortcut {}",
                binding.shortcut
            );
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_configs_keep_bindings_and_gain_default_growth() {
        let config: Config = toml::from_str(
            "version = 1\n[[bindings]]\naction = 'left'\nshortcut = 'Ctrl+Alt+Digit1'",
        )
        .unwrap();
        assert_eq!(
            config.growth,
            Growth {
                percent: 15,
                steps: 2
            }
        );
        assert_eq!(config.bindings[0].shortcut, "Ctrl+Alt+Digit1");
        assert_eq!(config.width_resize.percent, 25);
        let partial: Config =
            toml::from_str("version = 1\nbindings = []\n[growth]\npercent = 25").unwrap();
        assert_eq!(
            partial.growth,
            Growth {
                percent: 25,
                steps: 2
            }
        );
    }

    #[test]
    fn width_shortcuts_and_settings_are_valid_and_persist() {
        let mut config = Config::default();
        for (action, key) in [
            (Action::ShrinkWidth, "Minus"),
            (Action::ExpandWidth, "Equal"),
        ] {
            let binding = config.bindings.iter().find(|b| b.action == action).unwrap();
            let actual: HotKey = binding.shortcut.parse().unwrap();
            let expected: HotKey = format!("Ctrl+Alt+{key}").parse().unwrap();
            assert_eq!(actual.id(), expected.id());
        }
        for percent in [0, 100] {
            config.width_resize.percent = percent;
            assert!(config.validate().is_err());
        }
        config.width_resize.percent = 40;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().width_resize,
            config.width_resize
        );
    }

    #[test]
    fn fill_gap_uses_shifted_number_row_8() {
        let config = Config::default();
        let binding = config
            .bindings
            .iter()
            .find(|b| b.action == Action::FillGap)
            .unwrap();
        let actual: HotKey = binding.shortcut.parse().unwrap();
        let expected: HotKey = "Ctrl+Alt+Shift+Digit8".parse().unwrap();
        assert_eq!(actual.id(), expected.id());
        assert_ne!(actual.id(), "Ctrl+Alt+8".parse::<HotKey>().unwrap().id());
        config.validate().unwrap();
    }

    #[test]
    fn growth_limits_and_custom_settings_roundtrip() {
        let mut config = Config::default();
        for settings in [
            Growth {
                percent: 0,
                steps: 2,
            },
            Growth {
                percent: 101,
                steps: 2,
            },
            Growth {
                percent: 15,
                steps: 21,
            },
        ] {
            config.growth = settings;
            assert!(config.validate().is_err());
        }
        config.growth = Growth {
            percent: 25,
            steps: 4,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().growth, config.growth);
        config.growth.steps = 0;
        config.validate().unwrap();
    }
    #[test]
    fn defaults_roundtrip_and_duplicates_rejected() {
        let mut config = Config::default();
        config.validate().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        config.save(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().bindings.len(),
            Action::ALL.len()
        );
        config.bindings[1].shortcut = "Control+Alt+Digit1".into();
        assert!(config.validate().is_err());
    }
    #[test]
    fn invalid_config_is_not_silently_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "broken").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn defaults_use_number_row_keys() {
        let config = Config::default();
        for (action, digit) in [(Action::Center, 3), (Action::Top, 8), (Action::Bottom, 5)] {
            let binding = config.bindings.iter().find(|b| b.action == action).unwrap();
            assert_eq!(binding.shortcut, format!("Ctrl+Alt+{digit}"));
        }
        for binding in config.bindings {
            if !Action::POSITIONS.contains(&binding.action) {
                continue;
            }
            let number_row: HotKey = binding.shortcut.parse().unwrap();
            let numpad: HotKey = binding
                .shortcut
                .replace("Alt+", "Alt+Numpad")
                .parse()
                .unwrap();
            assert_ne!(numpad.id(), number_row.id());
        }
    }
}

#[cfg(test)]
mod focus_binding_tests {
    use super::*;
    #[test]
    fn old_configs_gain_vim_defaults_without_overriding_user_bindings() {
        let mut config = Config::default();
        for (action, key) in [
            (Action::FocusLeft, "H"),
            (Action::FocusDown, "J"),
            (Action::FocusUp, "K"),
            (Action::FocusRight, "L"),
        ] {
            assert_eq!(
                config
                    .bindings
                    .iter()
                    .find(|b| b.action == action)
                    .unwrap()
                    .shortcut,
                format!("Ctrl+Alt+{key}")
            );
        }
        config.bindings.retain(|b| !b.action.focuses());
        config.bindings[0].shortcut = "Ctrl+Alt+H".into();
        config.bindings.push(Binding {
            action: Action::FocusDown,
            shortcut: String::new(),
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.bindings[0].shortcut, "Ctrl+Alt+H");
        for action in [Action::FocusLeft, Action::FocusDown] {
            assert!(
                loaded
                    .bindings
                    .iter()
                    .find(|b| b.action == action)
                    .unwrap()
                    .shortcut
                    .is_empty()
            );
        }
        assert_eq!(
            loaded
                .bindings
                .iter()
                .find(|b| b.action == Action::FocusRight)
                .unwrap()
                .shortcut,
            "Ctrl+Alt+L"
        );
        loaded.save(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().bindings.len(),
            loaded.bindings.len()
        );
    }
}

#[cfg(test)]
mod focus_visibility_tests {
    use super::*;
    #[test]
    fn glow_migrates_roundtrips_and_rejects_invalid_colors() {
        let mut config: Config =
            toml::from_str("version = 1\nbindings = []\n[focus]\ninclude_obscured = true").unwrap();
        assert!(config.focus.glow_enabled);
        assert_eq!(config.focus.glow_rgb(), Some(0xf2f0e8));
        config.focus.glow_enabled = false;
        config.focus.glow_color = "#12aBcD".into();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().focus, config.focus);
        for invalid in ["red", "123456", "#fff", "#1234567", "#gg0000", "#é0000"] {
            config.focus.glow_color = invalid.into();
            assert!(config.validate().is_err(), "{invalid}");
        }
    }
    #[test]
    fn visibility_percentage_defaults_migrates_validates_and_roundtrips() {
        let old: Config = toml::from_str("version = 1\nbindings = []").unwrap();
        assert_eq!(old.focus.minimum_visible_percent, 10);
        assert_eq!(Config::default().focus.minimum_visible_percent, 10);
        for (settings, expected) in [
            ("glow_enabled = false", 10),
            ("include_obscured = true", 10),
            ("include_obscured = false", 50),
            ("include_obscured = false\nminimum_visible_percent = 25", 25),
        ] {
            let config: Config =
                toml::from_str(&format!("version = 1\nbindings = []\n[focus]\n{settings}"))
                    .unwrap();
            assert_eq!(config.focus.minimum_visible_percent, expected);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for value in [0, 10, 25, 50, 99] {
            let mut config = old.clone();
            config.focus.minimum_visible_percent = value;
            config.save(&path).unwrap();
            assert_eq!(
                Config::load(&path).unwrap().focus.minimum_visible_percent,
                value
            );
            assert!(
                !std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("include_obscured")
            );
        }
        for value in [100, u32::MAX] {
            let mut config = old.clone();
            config.focus.minimum_visible_percent = value;
            assert!(config.validate().is_err());
        }
    }
}

#[cfg(test)]
mod rebrand_tests {
    use super::*;

    #[test]
    fn existing_settings_survive_rebrand_and_new_location_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tt-pane/config.toml");
        let legacy = dir.path().join("stile/config.toml");
        assert_eq!(
            Config::existing_config_path(path.clone(), legacy.clone()).unwrap(),
            path
        );
        let mut config = Config::default();
        config.growth.percent = 27;
        config.bindings[0].shortcut.clear();
        config.save(&legacy).unwrap();
        let selected = Config::existing_config_path(path.clone(), legacy.clone()).unwrap();
        assert_eq!(selected, legacy);
        let loaded = Config::load(&selected).unwrap();
        assert_eq!(loaded.growth.percent, 27);
        assert!(loaded.bindings[0].shortcut.is_empty());
        Config::default().save(&path).unwrap();
        assert_eq!(
            Config::existing_config_path(path.clone(), legacy).unwrap(),
            path
        );
    }
}
