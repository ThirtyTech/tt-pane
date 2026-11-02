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
        let keys = ["1", "2", "8", "5", "7", "9", "4", "6", "3", "0", "-", "="];
        Self {
            version: 1,
            growth: Growth::default(),
            width_resize: WidthResize::default(),
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
        Ok(directories::ProjectDirs::from("", "", "stile")
            .context("Could not locate the user configuration directory")?
            .config_dir()
            .join("config.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config: Self = toml::from_str(&text).context("Invalid configuration TOML")?;
                config.validate()?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("Read {}", path.display())),
        }
    }

    pub fn validate(&self) -> Result<()> {
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
        assert_eq!(Config::load(&path).unwrap().bindings.len(), 12);
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
            if binding.action.adjusts_width() {
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
