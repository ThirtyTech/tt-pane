use crate::{
    config::Config,
    dock::{PickStep, Toward},
    geometry::Action,
    platform,
};
use anyhow::Result;
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
    /// What a dock pick mode key asks for.
    pub pick: Option<PickStep>,
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
                pick: None,
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
                    pick: None,
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

/// Resize and dock pick modes end by themselves after this long without a key,
/// so plain keys are never held hostage if the user walks away.
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

/// Plain keys grabbed only while dock pick mode is on. Digits pick a numbered
/// preview, the arrows and H/J/K/L step through the dock, and None ends the mode.
fn pick_keys() -> Vec<(HotKey, Option<PickStep>)> {
    const DIGITS: [Code; 9] = [
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    const NUMPAD: [Code; 9] = [
        Code::Numpad1,
        Code::Numpad2,
        Code::Numpad3,
        Code::Numpad4,
        Code::Numpad5,
        Code::Numpad6,
        Code::Numpad7,
        Code::Numpad8,
        Code::Numpad9,
    ];
    let plain = |code| HotKey::new(None, code);
    let slots = DIGITS
        .into_iter()
        .enumerate()
        .chain(NUMPAD.into_iter().enumerate())
        .map(|(slot, code)| (plain(code), Some(PickStep::Slot(slot))));
    // Arrows and H/J/K/L go by what's on screen; with Shift they move the
    // picked window that way instead.
    const BACK: [Code; 4] = [Code::ArrowLeft, Code::ArrowUp, Code::KeyH, Code::KeyK];
    const FORTH: [Code; 4] = [Code::ArrowRight, Code::ArrowDown, Code::KeyL, Code::KeyJ];
    let shift = |code| HotKey::new(Some(Modifiers::SHIFT), code);
    let directions = [(BACK, Toward::LeftOrUp), (FORTH, Toward::RightOrDown)]
        .into_iter()
        .flat_map(|(codes, toward)| {
            codes.into_iter().flat_map(move |code| {
                [
                    (plain(code), Some(PickStep::Toward(toward))),
                    (shift(code), Some(PickStep::Move(toward))),
                ]
            })
        });
    let steps = [
        (plain(Code::Tab), Some(PickStep::Next)),
        (shift(Code::Tab), Some(PickStep::Previous)),
        (plain(Code::Escape), None),
        (plain(Code::Enter), None),
    ];
    slots.chain(directions).chain(steps).collect()
}

struct PickMode {
    keys: Vec<(HotKey, Option<PickStep>)>,
    last: std::time::Instant,
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
    pick: Option<PickMode>,
    /// Glow color held around the focused window during resize mode.
    glow: Option<u32>,
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
            pick: None,
            glow: None,
        })
    }

    pub fn resizing(&self) -> bool {
        self.resize.is_some()
    }

    pub fn resizing_individually(&self) -> bool {
        self.resize.as_ref().is_some_and(|mode| mode.individual)
    }

    pub fn picking(&self) -> bool {
        self.pick.is_some()
    }

    /// Register a mode's plain keys. A user binding on the same key keeps
    /// working as configured, and a key another application owns is skipped.
    fn grab<T>(&self, keys: Vec<(HotKey, T)>) -> Vec<(HotKey, T)> {
        keys.into_iter()
            .filter(|(key, _)| !self.registered.iter().any(|(own, _)| own == key))
            .filter(|(key, _)| {
                self.manager
                    .as_ref()
                    .is_none_or(|manager| manager.register(*key).is_ok())
            })
            .collect()
    }

    fn release<T>(&self, keys: Vec<(HotKey, T)>) {
        if let Some(manager) = &self.manager {
            for (key, _) in keys {
                let _ = manager.unregister(key);
            }
        }
    }

    fn enter_pick(&mut self, now: std::time::Instant) {
        self.leave_resize();
        let keys = self.grab(pick_keys());
        self.pick = Some(PickMode { keys, last: now });
        #[cfg(windows)]
        platform::set_dock_picking(true);
    }

    /// End dock pick mode, such as when the dock turns out to be empty.
    pub fn leave_pick(&mut self) {
        if let Some(mode) = self.pick.take() {
            self.release(mode.keys);
            #[cfg(windows)]
            platform::set_dock_picking(false);
        }
    }

    fn enter_resize(&mut self, now: std::time::Instant) {
        self.leave_pick();
        let keys = self.grab(resize_keys());
        self.resize = Some(ResizeMode {
            keys,
            last: now,
            individual: false,
        });
        #[cfg(windows)]
        if self.manager.is_some()
            && let Some(color) = self.glow
        {
            platform::hold_focus_glow(color);
        }
    }

    fn leave_resize(&mut self) {
        if let Some(mode) = self.resize.take() {
            #[cfg(windows)]
            if self.manager.is_some() {
                platform::release_focus_glow();
            }
            self.release(mode.keys);
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
        if let Some(mode) = &mut self.pick
            && let Some(&(_, step)) = mode.keys.iter().find(|(key, _)| key.id() == id)
        {
            if state != HotKeyState::Pressed {
                return None;
            }
            mode.last = now;
            return match step {
                Some(step) => Some(Invocation {
                    action: Action::DockPick,
                    window,
                    include_hidden: false,
                    pick: Some(step),
                }),
                None => {
                    self.leave_pick();
                    None
                }
            };
        }
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
                        pick: None,
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
                    pick: None,
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
                pick: None,
            });
        }
        if action == Action::DockPick {
            if state != HotKeyState::Pressed {
                return None;
            }
            // Pressing it again steps to the next docked window.
            let step = match &mut self.pick {
                Some(mode) => {
                    mode.last = now;
                    PickStep::Next
                }
                None => {
                    self.enter_pick(now);
                    PickStep::Start
                }
            };
            return Some(Invocation {
                action,
                window,
                include_hidden: false,
                pick: Some(step),
            });
        }
        // Any other shortcut ends a mode before doing its own thing.
        if state == HotKeyState::Pressed {
            self.leave_resize();
            self.leave_pick();
        }
        if cfg!(windows) {
            self.gesture.event(id, action, state, now, window)
        } else {
            (state == HotKeyState::Pressed).then_some(Invocation {
                action,
                window: None,
                include_hidden: false,
                pick: None,
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
        if self
            .pick
            .as_ref()
            .is_some_and(|mode| now.duration_since(mode.last) >= RESIZE_IDLE)
        {
            self.leave_pick();
        }
        self.gesture.poll(now)
    }

    /// Register a complete set. A key another application owns is skipped and
    /// returned, so one conflict never disables the other shortcuts.
    pub fn replace(&mut self, config: &Config) -> Result<Vec<(Action, HotKey)>> {
        self.leave_resize();
        self.leave_pick();
        self.glow = config
            .focus
            .glow_enabled
            .then(|| config.focus.glow_rgb())
            .flatten();
        let mut next = configured_hotkeys(config)?;
        let mut taken = Vec::new();
        if let Some(manager) = &self.manager {
            // Keep shared keys registered, so swapping actions doesn't lose a grab.
            let registered = &self.registered;
            next.retain(|&(key, action)| {
                let ok =
                    registered.iter().any(|(old, _)| *old == key) || manager.register(key).is_ok();
                if !ok {
                    taken.push((action, key));
                }
                ok
            });
            for (key, _) in &self.registered {
                if !next.iter().any(|(new, _)| new == key) {
                    manager.unregister(*key)?;
                }
            }
        }
        self.gesture = FocusGesture::default();
        self.registered = next;
        Ok(taken)
    }

    pub fn suspend(&mut self) -> Result<()> {
        self.replace(&Config {
            version: 1,
            bindings: Vec::new(),
            ..Config::default()
        })
        .map(drop)
    }
}

/// Names the shortcuts `Shortcuts::replace` could not register, if any, as
/// `config` spells them.
pub fn taken_message(config: &Config, taken: &[(Action, HotKey)]) -> Option<String> {
    let names: Vec<_> = taken
        .iter()
        .map(|(action, key)| {
            let shortcut = config
                .bindings
                .iter()
                .find(|b| b.action == *action && b.shortcut.parse::<HotKey>().ok() == Some(*key))
                .map_or_else(|| key.to_string(), |b| b.shortcut.clone());
            format!("{} ({shortcut})", action.label())
        })
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "Could not register {}; another application may own {}. Your other shortcuts work.",
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" }
        )
    })
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        self.leave_resize();
        self.leave_pick();
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
            "Period" => "period",
            "Comma" => "comma",
            "Slash" => "slash",
            "Backslash" => "backslash",
            "Semicolon" => "semicolon",
            "Quote" => "apostrophe",
            "Backquote" => "grave",
            "BracketLeft" => "bracketleft",
            "BracketRight" => "bracketright",
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
                    include_hidden: false,
                    pick: None
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
                    include_hidden: true,
                    pick: None
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
                include_hidden: false,
                pick: None
            })
        );
        assert_eq!(gesture.poll(start + HOLD_DURATION), None);
        let mut shortcuts = Shortcuts {
            manager: None,
            registered: Vec::new(),
            gesture,
            resize: None,
            pick: None,
            glow: None,
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
            pick: None,
            glow: None,
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
    fn dock_pick_mode_maps_digits_and_steps_until_escape_another_shortcut_or_idle() {
        use std::time::Instant;
        let config = Config::default();
        let mut shortcuts = Shortcuts {
            manager: None,
            registered: configured_hotkeys(&config).unwrap(),
            gesture: FocusGesture::default(),
            resize: None,
            pick: None,
            glow: None,
        };
        let id = |code| HotKey::new(None, code).id();
        let pick = |invocation: Option<Invocation>| {
            let invocation = invocation.unwrap();
            assert_eq!(invocation.action, Action::DockPick);
            invocation.pick.unwrap()
        };
        let start = Instant::now();
        // Digits mean nothing until the mode starts.
        assert_eq!(
            shortcuts.event(id(Code::Digit1), HotKeyState::Pressed, start),
            None
        );
        let Some(toggle) = shortcuts
            .registered
            .iter()
            .find(|(_, a)| *a == Action::DockPick)
            .map(|(key, _)| key.id())
        else {
            return; // No default outside Windows.
        };
        assert_eq!(
            pick(shortcuts.event(toggle, HotKeyState::Pressed, start)),
            PickStep::Start
        );
        assert!(shortcuts.picking());
        for (code, step) in [
            (Code::Digit1, PickStep::Slot(0)),
            (Code::Digit9, PickStep::Slot(8)),
            (Code::Numpad3, PickStep::Slot(2)),
            (Code::ArrowLeft, PickStep::Toward(Toward::LeftOrUp)),
            (Code::KeyK, PickStep::Toward(Toward::LeftOrUp)),
            (Code::ArrowRight, PickStep::Toward(Toward::RightOrDown)),
            (Code::KeyJ, PickStep::Toward(Toward::RightOrDown)),
            (Code::Tab, PickStep::Next),
        ] {
            assert_eq!(
                pick(shortcuts.event(id(code), HotKeyState::Pressed, start)),
                step
            );
            assert_eq!(
                shortcuts.event(id(code), HotKeyState::Released, start),
                None
            );
        }
        let shifted = |code| HotKey::new(Some(Modifiers::SHIFT), code).id();
        assert_eq!(
            pick(shortcuts.event(shifted(Code::Tab), HotKeyState::Pressed, start)),
            PickStep::Previous
        );
        // Shift with an arrow or H/J/K/L moves the picked window.
        for (code, toward) in [
            (Code::ArrowLeft, Toward::LeftOrUp),
            (Code::ArrowUp, Toward::LeftOrUp),
            (Code::KeyH, Toward::LeftOrUp),
            (Code::ArrowRight, Toward::RightOrDown),
            (Code::ArrowDown, Toward::RightOrDown),
            (Code::KeyJ, Toward::RightOrDown),
        ] {
            assert_eq!(
                pick(shortcuts.event(shifted(code), HotKeyState::Pressed, start)),
                PickStep::Move(toward)
            );
        }
        // Pressing the shortcut again steps on instead of ending the mode.
        assert_eq!(
            pick(shortcuts.event(toggle, HotKeyState::Pressed, start)),
            PickStep::Next
        );
        assert!(shortcuts.picking());
        shortcuts.event(id(Code::Escape), HotKeyState::Pressed, start);
        assert!(!shortcuts.picking());
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        shortcuts.event(id(Code::Enter), HotKeyState::Pressed, start);
        assert!(!shortcuts.picking());
        // Another shortcut ends it and does its own thing.
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
        assert!(!shortcuts.picking());
        // Resize mode and pick mode replace each other.
        if let Some(resize) = shortcuts
            .registered
            .iter()
            .find(|(_, a)| *a == Action::ResizeMode)
            .map(|(key, _)| key.id())
        {
            shortcuts.event(toggle, HotKeyState::Pressed, start);
            shortcuts.event(resize, HotKeyState::Pressed, start);
            assert!(shortcuts.resizing() && !shortcuts.picking());
            shortcuts.event(toggle, HotKeyState::Pressed, start);
            assert!(!shortcuts.resizing() && shortcuts.picking());
            shortcuts.event(id(Code::Escape), HotKeyState::Pressed, start);
        }
        // Idling ends it; each key restarts the clock.
        shortcuts.event(toggle, HotKeyState::Pressed, start);
        shortcuts.event(
            id(Code::Digit2),
            HotKeyState::Pressed,
            start + RESIZE_IDLE / 2,
        );
        shortcuts.poll(start + RESIZE_IDLE);
        assert!(shortcuts.picking());
        shortcuts.poll(start + RESIZE_IDLE * 2);
        assert!(!shortcuts.picking());
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
    fn a_taken_key_leaves_the_other_shortcuts_registered() {
        use crate::config::Binding;
        let owner = GlobalHotKeyManager::new().unwrap();
        let blocked: HotKey = "Ctrl+Alt+Shift+F23".parse().unwrap();
        owner.register(blocked).unwrap();
        let original: HotKey = "Ctrl+Alt+Shift+F22".parse().unwrap();
        let added: HotKey = "Ctrl+Alt+Shift+F21".parse().unwrap();
        let mut shortcuts = Shortcuts::new().unwrap();
        let config = Config {
            version: 1,
            bindings: vec![Binding {
                action: Action::Left,
                shortcut: original.to_string(),
            }],
            ..Config::default()
        };
        assert!(shortcuts.replace(&config).unwrap().is_empty());
        // The taken key comes before a free one, as Toggle float's default
        // came before the tiling keys after it.
        let mut candidate = config.clone();
        candidate.bindings.push(Binding {
            action: Action::Right,
            shortcut: blocked.to_string(),
        });
        candidate.bindings.push(Binding {
            action: Action::Center,
            shortcut: added.to_string(),
        });
        let taken = shortcuts.replace(&candidate).unwrap();
        assert_eq!(taken, vec![(Action::Right, blocked)]);
        assert!(
            taken_message(&candidate, &taken)
                .unwrap()
                .contains(&format!("Right half ({blocked})"))
        );
        assert_eq!(shortcuts.action(original.id()), Some(Action::Left));
        assert_eq!(shortcuts.action(added.id()), Some(Action::Center));
        assert_eq!(shortcuts.action(blocked.id()), None);
        assert!(owner.register(original).is_err());
        assert!(owner.register(added).is_err());
        shortcuts.suspend().unwrap();
        owner.register(original).unwrap();
        owner.unregister(original).unwrap();
        owner.unregister(blocked).unwrap();
    }
}
