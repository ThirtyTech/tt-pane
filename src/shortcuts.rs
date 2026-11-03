use crate::{config::Config, geometry::Action, platform};
use anyhow::{Context, Result};
use global_hotkey::{
    GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};

fn configured_hotkeys(config: &Config) -> Result<Vec<(HotKey, Action)>> {
    config.validate()?;
    let mut keys = config
        .bindings
        .iter()
        .filter(|b| !b.shortcut.is_empty())
        .map(|b| Ok((b.shortcut.parse::<HotKey>()?, b.action)))
        .collect::<Result<Vec<_>>>()?;
    // The +/= key works with or without Shift for width expansion, unless the
    // shifted combination was explicitly assigned to another action by the user.
    let aliases = keys
        .iter()
        .filter(|(key, action)| {
            *action == Action::ExpandWidth
                && key.key == Code::Equal
                && !key.mods.contains(Modifiers::SHIFT)
        })
        .map(|(key, _)| HotKey::new(Some(key.mods | Modifiers::SHIFT), Code::Equal))
        .collect::<Vec<_>>();
    for alias in aliases {
        if !keys.iter().any(|(key, _)| key.id() == alias.id()) {
            keys.push((alias, Action::ExpandWidth));
        }
    }
    Ok(keys)
}

const HOLD_DURATION: std::time::Duration = std::time::Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation {
    pub action: Action,
    pub window: Option<u64>,
    pub include_hidden: bool,
}

struct HeldFocus {
    id: u32,
    started: std::time::Instant,
    invocation: Invocation,
    fired: bool,
}

#[derive(Default)]
struct FocusGesture {
    held: Option<HeldFocus>,
}

impl FocusGesture {
    fn event(
        &mut self,
        id: u32,
        action: Action,
        state: HotKeyState,
        now: std::time::Instant,
        window: Option<u64>,
    ) -> Option<Invocation> {
        if state == HotKeyState::Released {
            if self.held.as_ref().is_some_and(|held| held.id == id) {
                let held = self.held.take().unwrap();
                return (!held.fired).then_some(Invocation {
                    include_hidden: now.duration_since(held.started) >= HOLD_DURATION,
                    ..held.invocation
                });
            }
            return None;
        }
        if !matches!(action, Action::FocusLeft | Action::FocusRight) {
            self.held = None;
            return Some(Invocation {
                action,
                window,
                include_hidden: false,
            });
        }
        // Ignore native repeat presses without restarting the hold timer.
        if !self.held.as_ref().is_some_and(|held| held.id == id) {
            self.held = Some(HeldFocus {
                id,
                started: now,
                invocation: Invocation {
                    action,
                    window,
                    include_hidden: false,
                },
                fired: false,
            });
        }
        None
    }

    fn poll(&mut self, now: std::time::Instant) -> Option<Invocation> {
        let held = self.held.as_mut()?;
        if held.fired || now.duration_since(held.started) < HOLD_DURATION {
            return None;
        }
        held.fired = true;
        Some(Invocation {
            include_hidden: true,
            ..held.invocation
        })
    }
}

/// Lives on the GPUI event-loop thread (required by macOS and Windows).
pub struct Shortcuts {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, Action)>,
    gesture: FocusGesture,
}

impl Shortcuts {
    pub fn new() -> Result<Self> {
        Ok(Self {
            manager: if platform::is_wayland() {
                None
            } else {
                Some(GlobalHotKeyManager::new()?)
            },
            registered: Vec::new(),
            gesture: FocusGesture::default(),
        })
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        self.registered
            .iter()
            .find(|(key, _)| key.id() == id)
            .map(|(_, action)| *action)
    }

    pub fn event(
        &mut self,
        id: u32,
        state: HotKeyState,
        now: std::time::Instant,
    ) -> Option<Invocation> {
        let action = self.action(id)?;
        if cfg!(windows) {
            self.gesture
                .event(id, action, state, now, platform::foreground_handle())
        } else {
            (state == HotKeyState::Pressed).then_some(Invocation {
                action,
                window: None,
                include_hidden: false,
            })
        }
    }

    pub fn poll(&mut self, now: std::time::Instant) -> Option<Invocation> {
        self.gesture.poll(now)
    }

    /// Register a complete set, rolling back on OS shortcut conflicts.
    pub fn replace(&mut self, config: &Config) -> Result<()> {
        let next = configured_hotkeys(config)?;
        if let Some(manager) = &self.manager {
            // Keep shared keys registered, so swapping actions doesn't lose a grab.
            let added: Vec<_> = next
                .iter()
                .filter(|(k, _)| !self.registered.iter().any(|(old, _)| old == k))
                .copied()
                .collect();
            let mut acquired = Vec::new();
            for (key, action) in added {
                if let Err(error) = manager.register(key) {
                    for key in acquired {
                        let _ = manager.unregister(key);
                    }
                    return Err(error).with_context(|| {
                        format!(
                            "Could not register {} ({key}); another application may own it",
                            action.label()
                        )
                    });
                }
                acquired.push(key);
            }
            for (key, _) in &self.registered {
                if !next.iter().any(|(new, _)| new == key) {
                    manager.unregister(*key)?;
                }
            }
        }
        self.gesture = FocusGesture::default();
        self.registered = next;
        Ok(())
    }

    pub fn suspend(&mut self) -> Result<()> {
        self.replace(&Config {
            version: 1,
            bindings: Vec::new(),
            ..Config::default()
        })
    }
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        if let Some(manager) = &self.manager {
            for (key, _) in &self.registered {
                let _ = manager.unregister(*key);
            }
        }
    }
}

/// Wayland shortcuts belong to the compositor. Export, never silently alter user config.
pub fn wayland_bindings(
    config: &Config,
    compositor: &str,
    executable: &std::path::Path,
    config_path: Option<&std::path::Path>,
) -> Result<String> {
    use global_hotkey::hotkey::Modifiers;
    config.validate()?;
    anyhow::ensure!(
        matches!(compositor, "sway" | "hyprland"),
        "Choose sway or hyprland"
    );
    let exe = executable.to_string_lossy();
    anyhow::ensure!(
        !exe.contains(['\n', '\r', '"', '$', '`', '\\']),
        "Executable path cannot safely be exported"
    );
    let config_arg = if let Some(path) = config_path {
        let path = path.to_string_lossy();
        anyhow::ensure!(
            !path.contains(['\n', '\r', '"', '$', '`', '\\', ',', '#']),
            "Configuration path cannot safely be exported"
        );
        format!(" --config \"{path}\"")
    } else {
        String::new()
    };
    let mut text = String::from(
        "# Generated by tt-pane. Include in your compositor configuration, then reload.\n",
    );
    for (key, action) in configured_hotkeys(config)? {
        let raw = key.key.to_string();
        let symbol = if let Some(n) = raw.strip_prefix("Digit") {
            n.to_string()
        } else if let Some(n) = raw.strip_prefix("Numpad")
            && n.len() == 1
            && n.as_bytes()[0].is_ascii_digit()
        {
            format!("KP_{n}")
        } else if let Some(k) = raw.strip_prefix("Key") {
            k.to_lowercase()
        } else {
            match raw.as_str() {
                "ArrowLeft" => "Left",
                "ArrowRight" => "Right",
                "ArrowUp" => "Up",
                "ArrowDown" => "Down",
                "Space" => "space",
                "Enter" => "Return",
                "Escape" => "Escape",
                "Backspace" => "BackSpace",
                "Delete" => "Delete",
                "Tab" => "Tab",
                "Home" => "Home",
                "End" => "End",
                "PageUp" => "Prior",
                "PageDown" => "Next",
                "Minus" => "minus",
                "Equal" => "equal",
                "NumpadSubtract" => "KP_Subtract",
                "NumpadAdd" => "KP_Add",
                "NumpadMultiply" => "KP_Multiply",
                k if k.starts_with('F') && k[1..].parse::<u8>().is_ok() => k,
                _ => anyhow::bail!("Wayland export does not yet support key {raw}"),
            }
            .to_string()
        };
        let mut mods = Vec::new();
        if key.mods.contains(Modifiers::CONTROL) {
            mods.push("Ctrl");
        }
        if key.mods.contains(Modifiers::ALT) {
            mods.push(if compositor == "sway" { "Mod1" } else { "Alt" });
        }
        if key.mods.contains(Modifiers::SHIFT) {
            mods.push("Shift");
        }
        if key.mods.contains(Modifiers::META) {
            mods.push(if compositor == "sway" {
                "Mod4"
            } else {
                "Super"
            });
        }
        if compositor == "sway" {
            text.push_str(&format!(
                "bindsym {}+{} exec \"{}\"{} apply {}\n",
                mods.join("+"),
                symbol,
                exe,
                config_arg,
                action.name()
            ));
        } else {
            text.push_str(&format!(
                "bind = {}, {}, exec, \"{}\"{} apply {}\n",
                mods.join(" "),
                symbol,
                exe,
                config_arg,
                action.name()
            ));
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn horizontal_tap_and_hold_are_mutually_exclusive_and_repeat_safe() {
        use std::time::{Duration, Instant};
        for action in [Action::FocusLeft, Action::FocusRight] {
            let start = Instant::now();
            let mut gesture = FocusGesture::default();
            assert_eq!(
                gesture.event(1, action, HotKeyState::Pressed, start, Some(42)),
                None
            );
            assert_eq!(gesture.poll(start + Duration::from_millis(399)), None);
            assert_eq!(
                gesture.event(
                    1,
                    action,
                    HotKeyState::Released,
                    start + Duration::from_millis(100),
                    Some(99)
                ),
                Some(Invocation {
                    action,
                    window: Some(42),
                    include_hidden: false
                })
            );
            assert_eq!(gesture.poll(start + HOLD_DURATION), None);
            gesture.event(1, action, HotKeyState::Pressed, start, Some(42));
            gesture.event(
                1,
                action,
                HotKeyState::Pressed,
                start + Duration::from_millis(300),
                Some(99),
            );
            assert_eq!(
                gesture.poll(start + HOLD_DURATION),
                Some(Invocation {
                    action,
                    window: Some(42),
                    include_hidden: true
                })
            );
            assert_eq!(gesture.poll(start + HOLD_DURATION * 2), None);
            assert_eq!(
                gesture.event(
                    1,
                    action,
                    HotKeyState::Released,
                    start + HOLD_DURATION * 2,
                    Some(99)
                ),
                None
            );
            // A release received before the next timer tick must still count as a hold.
            gesture.event(1, action, HotKeyState::Pressed, start, Some(42));
            assert!(
                gesture
                    .event(
                        1,
                        action,
                        HotKeyState::Released,
                        start + HOLD_DURATION,
                        None
                    )
                    .unwrap()
                    .include_hidden
            );
        }
    }

    #[test]
    fn other_actions_and_rebinding_cancel_pending_holds() {
        let start = std::time::Instant::now();
        let mut gesture = FocusGesture::default();
        gesture.event(1, Action::FocusLeft, HotKeyState::Pressed, start, Some(42));
        assert_eq!(
            gesture.event(2, Action::FocusDown, HotKeyState::Pressed, start, Some(42)),
            Some(Invocation {
                action: Action::FocusDown,
                window: Some(42),
                include_hidden: false
            })
        );
        assert_eq!(gesture.poll(start + HOLD_DURATION), None);
        let mut shortcuts = Shortcuts {
            manager: None,
            registered: Vec::new(),
            gesture,
        };
        shortcuts
            .gesture
            .event(1, Action::FocusRight, HotKeyState::Pressed, start, Some(42));
        shortcuts.suspend().unwrap();
        assert_eq!(shortcuts.poll(start + HOLD_DURATION), None);
    }

    #[test]
    fn plus_alias_respects_explicit_bindings_and_exports() {
        let mut config = Config::default();
        let plus: HotKey = "Ctrl+Alt+Shift+Equal".parse().unwrap();
        assert!(
            configured_hotkeys(&config)
                .unwrap()
                .contains(&(plus, Action::ExpandWidth))
        );
        for compositor in ["sway", "hyprland"] {
            let text = wayland_bindings(
                &config,
                compositor,
                std::path::Path::new("/usr/bin/tt-pane"),
                None,
            )
            .unwrap();
            assert!(text.contains("minus"));
            assert!(text.contains("equal"));
            assert_eq!(text.matches("apply expand-width").count(), 2);
        }
        config
            .bindings
            .iter_mut()
            .find(|b| b.action == Action::Center)
            .unwrap()
            .shortcut = "Ctrl+Alt+Shift+Equal".into();
        let keys = configured_hotkeys(&config).unwrap();
        assert!(keys.contains(&(plus, Action::Center)));
        assert!(!keys.contains(&(plus, Action::ExpandWidth)));
    }
    #[test]
    fn exports_default_compositor_shortcuts() {
        let config = Config::default();
        let path = std::path::Path::new("/usr/local/bin/tt-pane");
        assert!(
            wayland_bindings(&config, "sway", path, None)
                .unwrap()
                .contains("bindsym Ctrl+Mod1+7 exec \"/usr/local/bin/tt-pane\" apply top-left")
        );
        assert!(
            wayland_bindings(&config, "hyprland", path, None)
                .unwrap()
                .contains("bind = Ctrl Alt, 5, exec, \"/usr/local/bin/tt-pane\" apply bottom")
        );
    }

    #[test]
    fn export_preserves_custom_configuration_for_growth() {
        for compositor in ["sway", "hyprland"] {
            let text = wayland_bindings(
                &Config::default(),
                compositor,
                std::path::Path::new("/usr/bin/tt-pane"),
                Some(std::path::Path::new("/home/me/my settings.toml")),
            )
            .unwrap();
            assert!(text.contains(
                "\"/usr/bin/tt-pane\" --config \"/home/me/my settings.toml\" apply left"
            ));
            assert!(
                wayland_bindings(
                    &Config::default(),
                    compositor,
                    std::path::Path::new("/usr/bin/tt-pane"),
                    Some(std::path::Path::new("/home/me/$oops.toml"))
                )
                .is_err()
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_conflict_keeps_previous_shortcuts_registered() {
        use crate::config::Binding;
        let owner = GlobalHotKeyManager::new().unwrap();
        let blocked: HotKey = "Ctrl+Alt+Shift+F23".parse().unwrap();
        owner.register(blocked).unwrap();
        let original: HotKey = "Ctrl+Alt+Shift+F22".parse().unwrap();
        let mut shortcuts = Shortcuts::new().unwrap();
        let config = Config {
            version: 1,
            bindings: vec![Binding {
                action: Action::Left,
                shortcut: original.to_string(),
            }],
            ..Config::default()
        };
        shortcuts.replace(&config).unwrap();
        let mut candidate = config.clone();
        candidate.bindings.push(Binding {
            action: Action::Right,
            shortcut: blocked.to_string(),
        });
        assert!(shortcuts.replace(&candidate).is_err());
        assert_eq!(shortcuts.action(original.id()), Some(Action::Left));
        assert_eq!(shortcuts.action(blocked.id()), None);
        assert!(owner.register(original).is_err());
        shortcuts.suspend().unwrap();
        owner.register(original).unwrap();
        owner.unregister(original).unwrap();
        owner.unregister(blocked).unwrap();
    }
}
