use crate::{config::Config, geometry::Action, platform};
use anyhow::{Context, Result};
use global_hotkey::{
    GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};

pub(crate) fn configured_hotkeys(config: &Config) -> Result<Vec<(HotKey, Action)>> {
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

/// Resize mode ends by itself after this long without a key, so plain letters
/// are never held hostage if the user walks away.
const RESIZE_IDLE: std::time::Duration = std::time::Duration::from_secs(10);

/// Plain keys grabbed only while resize mode is on. None ends the mode, and
/// ResizeMode (I) switches between shared edges and the window alone.
fn resize_keys() -> Vec<(HotKey, Option<Action>)> {
    [
        (Code::KeyH, Some(Action::ResizeLeft)),
        (Code::ArrowLeft, Some(Action::ResizeLeft)),
        (Code::KeyJ, Some(Action::ResizeDown)),
        (Code::ArrowDown, Some(Action::ResizeDown)),
        (Code::KeyK, Some(Action::ResizeUp)),
        (Code::ArrowUp, Some(Action::ResizeUp)),
        (Code::KeyL, Some(Action::ResizeRight)),
        (Code::ArrowRight, Some(Action::ResizeRight)),
        (Code::KeyI, Some(Action::ResizeMode)),
        (Code::Escape, None),
        (Code::Enter, None),
    ]
    .into_iter()
    .map(|(code, action)| (HotKey::new(None, code), action))
    .collect()
}

struct ResizeMode {
    keys: Vec<(HotKey, Option<Action>)>,
    last: std::time::Instant,
    individual: bool,
}

/// In individual mode the divider moves become sizes of the window alone.
fn individual(action: Action) -> Action {
    match action {
        Action::ResizeLeft => Action::SizeNarrower,
        Action::ResizeRight => Action::SizeWider,
        Action::ResizeUp => Action::SizeShorter,
        Action::ResizeDown => Action::SizeTaller,
        other => other,
    }
}

/// Lives on the GPUI event-loop thread (required by macOS and Windows).
pub struct Shortcuts {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, Action)>,
    gesture: FocusGesture,
    resize: Option<ResizeMode>,
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
            resize: None,
        })
    }

    pub fn resizing(&self) -> bool {
        self.resize.is_some()
    }

    pub fn resizing_individually(&self) -> bool {
        self.resize.as_ref().is_some_and(|mode| mode.individual)
    }

    fn enter_resize(&mut self, now: std::time::Instant) {
        let keys = resize_keys()
            .into_iter()
            // A user binding on the same key keeps working as configured.
            .filter(|(key, _)| !self.registered.iter().any(|(own, _)| own == key))
            .filter(|(key, _)| {
                self.manager
                    .as_ref()
                    .is_none_or(|manager| manager.register(*key).is_ok())
            })
            .collect();
        self.resize = Some(ResizeMode {
            keys,
            last: now,
            individual: false,
        });
    }

    fn leave_resize(&mut self) {
        if let Some(mode) = self.resize.take()
            && let Some(manager) = &self.manager
        {
            for (key, _) in mode.keys {
                let _ = manager.unregister(key);
            }
        }
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
        #[cfg(windows)]
        let window = platform::foreground_handle();
        #[cfg(not(windows))]
        let window = None;
        if let Some(mode) = &mut self.resize
            && let Some(&(_, step)) = mode.keys.iter().find(|(key, _)| key.id() == id)
        {
            if state != HotKeyState::Pressed {
                return None;
            }
            mode.last = now;
            return match step {
                Some(Action::ResizeMode) => {
                    mode.individual = !mode.individual;
                    Some(Invocation {
                        action: Action::ResizeMode,
                        window,
                        include_hidden: false,
                    })
                }
                Some(action) => Some(Invocation {
                    action: if mode.individual {
                        individual(action)
                    } else {
                        action
                    },
                    window,
                    include_hidden: false,
                }),
                None => {
                    self.leave_resize();
                    None
                }
            };
        }
        let action = self.action(id)?;
        if action == Action::ResizeMode {
            if state != HotKeyState::Pressed {
                return None;
            }
            if self.resize.is_some() {
                self.leave_resize();
                return None;
            }
            self.enter_resize(now);
            return Some(Invocation {
                action,
                window,
                include_hidden: false,
            });
        }
        // Any other shortcut ends resize mode before doing its own thing.
        if state == HotKeyState::Pressed {
            self.leave_resize();
        }
        if cfg!(windows) {
            self.gesture.event(id, action, state, now, window)
        } else {
            (state == HotKeyState::Pressed).then_some(Invocation {
                action,
                window: None,
                include_hidden: false,
            })
        }
    }

    pub fn poll(&mut self, now: std::time::Instant) -> Option<Invocation> {
        if self
            .resize
            .as_ref()
            .is_some_and(|mode| now.duration_since(mode.last) >= RESIZE_IDLE)
        {
            self.leave_resize();
        }
        self.gesture.poll(now)
    }

    /// Register a complete set, rolling back on OS shortcut conflicts.
    pub fn replace(&mut self, config: &Config) -> Result<()> {
        self.leave_resize();
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
        self.leave_resize();
        if let Some(manager) = &self.manager {
            for (key, _) in &self.registered {
                let _ = manager.unregister(*key);
            }
        }
    }
}

/// X keysym name for a key, as GNOME accelerators expect.
#[cfg(target_os = "linux")]
pub(crate) fn keysym(key: Code) -> Result<String> {
    let raw = key.to_string();
    Ok(if let Some(n) = raw.strip_prefix("Digit") {
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
            _ => anyhow::bail!("Wayland shortcuts do not yet support key {raw}"),
        }
        .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_default_binding_parses_as_a_global_hotkey() {
        let config = Config::default();
        let keys = configured_hotkeys(&config).unwrap();
        for binding in config.bindings.iter().filter(|b| !b.shortcut.is_empty()) {
            assert!(
                keys.iter().any(|(_, action)| *action == binding.action),
                "{} ({}) did not register",
                binding.action.label(),
                binding.shortcut
            );
        }
    }

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
            resize: None,
        };
        shortcuts
            .gesture
            .event(1, Action::FocusRight, HotKeyState::Pressed, start, Some(42));
        shortcuts.suspend().unwrap();
        assert_eq!(shortcuts.poll(start + HOLD_DURATION), None);
    }

    #[test]
    fn resize_mode_maps_hjkl_until_escape_another_shortcut_or_idle() {
        use std::time::Instant;
        let config = Config::default();
        let mut shortcuts = Shortcuts {
            manager: None,
            registered: configured_hotkeys(&config).unwrap(),
            gesture: FocusGesture::default(),
            resize: None,
        };
        let id = |code| HotKey::new(None, code).id();
        let start = Instant::now();
        // Plain letters mean nothing until the mode starts.
        assert_eq!(
            shortcuts.event(id(Code::KeyH), HotKeyState::Pressed, start),
            None
        );
        let Some(toggle) = shortcuts
            .registered
            .iter()
            .find(|(_, a)| *a == Action::ResizeMode)
            .map(|(key, _)| key.id())
        else {
            return; // No default outside Windows.
        };
        let entered = shortcuts
            .event(toggle, HotKeyState::Pressed, start)
            .unwrap();
        assert_eq!(entered.action, Action::ResizeMode);
        for (code, action) in [
            (Code::KeyH, Action::ResizeLeft),
            (Code::KeyJ, Action::ResizeDown),
            (Code::KeyK, Action::ResizeUp),
            (Code::KeyL, Action::ResizeRight),
            (Code::ArrowLeft, Action::ResizeLeft),
        ] {
            let step = shortcuts.event(id(code), HotKeyState::Pressed, start);
            assert_eq!(step.unwrap().action, action);
            assert_eq!(
                shortcuts.event(id(code), HotKeyState::Released, start),
                None
            );
        }
        // I switches to sizing the window alone, and back.
        shortcuts.event(id(Code::KeyI), HotKeyState::Pressed, start);
        assert!(shortcuts.resizing_individually());
        for (code, action) in [
            (Code::KeyH, Action::SizeNarrower),
            (Code::KeyJ, Action::SizeTaller),
            (Code::KeyK, Action::SizeShorter),
            (Code::KeyL, Action::SizeWider),
        ] {
            let step = shortcuts.event(id(code), HotKeyState::Pressed, start);
            assert_eq!(step.unwrap().action, action);
        }
        shortcuts.event(id(Code::KeyI), HotKeyState::Pressed, start);
        assert!(!shortcuts.resizing_individually());
        shortcuts.event(id(Code::Escape), HotKeyState::Pressed, start);
        assert!(!shortcuts.resizing());
        // Pressing the shortcut again, another shortcut, or idling also ends it.
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        assert!(!shortcuts.resizing());
        let other = shortcuts
            .registered
            .iter()
            .find(|(_, a)| *a == Action::Left)
            .unwrap()
            .0
            .id();
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        assert_eq!(
            shortcuts
                .event(other, HotKeyState::Pressed, start)
                .unwrap()
                .action,
            Action::Left
        );
        assert!(!shortcuts.resizing());
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        shortcuts.poll(start + RESIZE_IDLE / 2);
        assert!(shortcuts.resizing());
        shortcuts.poll(start + RESIZE_IDLE);
        assert!(!shortcuts.resizing());
    }

    #[test]
    fn plus_alias_respects_explicit_bindings() {
        let mut config = Config::default();
        let plus: HotKey = "Ctrl+Alt+Shift+Equal".parse().unwrap();
        assert!(
            configured_hotkeys(&config)
                .unwrap()
                .contains(&(plus, Action::ExpandWidth))
        );
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
