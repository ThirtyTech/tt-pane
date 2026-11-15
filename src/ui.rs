use crate::{
    config::{Binding, CenterLayout, Config, TrayIconStyle},
    geometry::Action,
    platform,
    shortcuts::Shortcuts,
};
use global_hotkey::GlobalHotKeyEvent;
use gpui::{prelude::*, *};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const PREVIEW_AREA: crate::geometry::Rect = crate::geometry::Rect {
    x: 0,
    y: 0,
    width: 232,
    height: 144,
};
const PREVIEW_DURATION: Duration = Duration::from_millis(180);

struct PositionPreview {
    action: Action,
    placement: Action,
    from: [f32; 4],
    to: [f32; 4],
    started: Option<Instant>,
}

impl PositionPreview {
    fn target(action: Action) -> [f32; 4] {
        let rect = action.target(PREVIEW_AREA, PREVIEW_AREA);
        [
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ]
    }

    fn new(action: Action) -> Self {
        Self {
            action,
            placement: action,
            from: Self::target(action),
            to: Self::target(action),
            started: None,
        }
    }

    fn sample(&self, now: Instant) -> ([f32; 4], bool) {
        let target = self.to;
        let Some(started) = self.started else {
            return (target, false);
        };
        let progress =
            now.saturating_duration_since(started).as_secs_f32() / PREVIEW_DURATION.as_secs_f32();
        if progress >= 1.0 {
            return (target, false);
        }
        let eased = ease_in_out(progress);
        (
            std::array::from_fn(|i| self.from[i] + (target[i] - self.from[i]) * eased),
            true,
        )
    }

    fn select(&mut self, action: Action, percent: u32, now: Instant) {
        // Only fixed placements have an anchor. Contextual actions must never
        // replace the remembered snap used by width and gap previews.
        if Action::POSITIONS.contains(&action) {
            self.placement = action;
        }
        let target = if action.focuses() || action.is_tiling() {
            Self::target(self.placement)
        } else if action.adjusts_pair() {
            let base = Action::BottomLeft.target(PREVIEW_AREA, PREVIEW_AREA);
            let neighbor = Action::TopLeft.target(PREVIEW_AREA, PREVIEW_AREA);
            let (rect, _) =
                crate::geometry::resize_vertical_pair(base, neighbor, action == Action::ResizeUp)
                    .unwrap();
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        } else if action == Action::FillGap {
            let base = self.placement.target(PREVIEW_AREA, PREVIEW_AREA);
            let reduced = base.adjust_width(
                PREVIEW_AREA,
                self.placement.horizontal_anchor().unwrap(),
                -(percent as i32),
            );
            let gap = base.freed_width_gap(reduced).unwrap_or(base);
            [
                gap.x as f32,
                gap.y as f32,
                gap.width as f32,
                gap.height as f32,
            ]
        } else if action.adjusts_width() {
            let base = self.placement.target(PREVIEW_AREA, PREVIEW_AREA);
            let rect = base.adjust_width(
                PREVIEW_AREA,
                self.placement.horizontal_anchor().unwrap(),
                percent as i32 * if action == Action::ShrinkWidth { -1 } else { 1 },
            );
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        } else {
            Self::target(action)
        };
        if action != self.action || target != self.to {
            // Retarget from the in-flight position rather than jumping to its endpoint.
            self.from = self.sample(now).0;
            self.action = action;
            self.to = target;
            self.started = Some(now);
        }
    }
}

struct Settings {
    draft: Config,
    active: Config,
    path: PathBuf,
    shortcuts: Option<Shortcuts>,
    /// Active shortcuts another application owns, flagged in their rows.
    taken: Vec<(Action, global_hotkey::hotkey::HotKey)>,
    recording: Option<usize>,
    focus: FocusHandle,
    status: String,
    error: bool,
    preview: PositionPreview,
    tab: Tab,
    /// The shortcut row under the mouse, which shows its unassign icon.
    hovered_binding: Option<usize>,
    window: Option<WindowHandle<Settings>>,
    _tray: crate::tray::Tray,
    update: UpdateState,
}

#[derive(Default)]
struct UpdateState {
    schedule: crate::update::CheckSchedule,
    installing: bool,
    pending: Option<std::sync::mpsc::Receiver<anyhow::Result<UpdateResult>>>,
    release: Option<crate::update::Release>,
    message: String,
}

enum UpdateResult {
    Checked(Option<crate::update::Release>),
    Restart,
}

impl Settings {
    fn check_update(&mut self, cx: &mut Context<Self>) {
        if self.update.pending.is_some() {
            return;
        }
        self.update.schedule.started(Instant::now());
        self.update.message = "Checking for updates…".into();
        self._tray.update.set_enabled(false);
        self._tray.update.set_text("Checking for updates…");
        let (sender, receiver) = std::sync::mpsc::channel();
        self.update.pending = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(crate::update::check().map(UpdateResult::Checked));
        });
        cx.notify();
    }

    fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.update.pending.is_some() {
            return;
        }
        let Some(release) = self.update.release.clone() else {
            return;
        };
        if serde_json::to_value(&self.draft).ok() != serde_json::to_value(&self.active).ok() {
            self.update.message =
                "Save your settings changes before updating and restarting.".into();
            cx.notify();
            return;
        }
        if self.recording.take().is_some()
            && let Err(error) = self.resume()
        {
            self.update.message = format!("Could not resume shortcuts before updating: {error:#}");
            cx.notify();
            return;
        }
        self.update.message = format!(
            "Downloading and verifying {}. tt-pane will restart when ready…",
            release.tag
        );
        self.update.installing = true;
        self._tray.update.set_enabled(false);
        self._tray.update.set_text("Updating…");
        let config = self.path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.update.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = crate::update::prepare(&release, &config)
                .and_then(|prepared| prepared.launch())
                .map(|()| UpdateResult::Restart);
            let _ = sender.send(result);
        });
        cx.notify();
    }

    fn poll_update(&mut self, cx: &mut Context<Self>) {
        use std::sync::mpsc::TryRecvError;
        if self
            .update
            .schedule
            .due(Instant::now(), self.update.pending.is_some())
        {
            self.check_update(cx);
        }
        let Some(receiver) = &self.update.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Update worker stopped unexpectedly"))
            }
        };
        self.update.pending = None;
        self.update.installing = false;
        match result {
            Ok(UpdateResult::Restart) => {
                cx.quit();
                return;
            }
            Ok(UpdateResult::Checked(Some(release))) => {
                self.update.message = format!(
                    "{} is available (current: {}). Choose Update and restart to install it.",
                    release.tag,
                    env!("CARGO_PKG_VERSION")
                );
                self.update.release = Some(release);
            }
            Ok(UpdateResult::Checked(None)) => {
                self.update.release = None;
                self.update.message =
                    format!("tt-pane {} is up to date.", env!("CARGO_PKG_VERSION"));
            }
            Err(error) => self.update.message = format!("Update failed: {error:#}"),
        }
        self._tray.update.set_enabled(true);
        self._tray
            .update
            .set_text(if self.update.release.is_some() {
                "Update and restart…"
            } else {
                "Check for updates…"
            });
        cx.notify();
    }

    fn message(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.status = text.into();
        self.error = error;
        // With Settings closed the status line is invisible; a terminal launch
        // should still see what went wrong.
        if error {
            eprintln!("tt-pane: {}", self.status);
        }
        let _ = self._tray.icon.set_tooltip(Some(if error {
            "tt-pane — action failed; open Settings for details"
        } else {
            "tt-pane — window shortcuts"
        }));
        cx.notify();
    }

    fn invoke(&mut self, invocation: crate::shortcuts::Invocation, cx: &mut Context<Self>) {
        if invocation.action == Action::ResizeMode {
            // Shortcuts holds the glow for as long as the mode lasts.
            let individual = self
                .shortcuts
                .as_ref()
                .is_some_and(|s| s.resizing_individually());
            self.message(
                if individual {
                    "Resize mode, this window only: H/L narrower/wider, K/J shorter/taller · I shares edges · Esc finishes"
                } else {
                    "Resize mode: H/J/K/L or arrows move shared edges · I sizes this window only · Esc finishes"
                },
                false,
                cx,
            );
            return;
        }
        match crate::positioning::apply_gesture(
            &self.active,
            &self.path,
            invocation.action,
            invocation.window,
            invocation.include_hidden,
        ) {
            Ok(()) => self.message(
                format!("Positioned: {}", invocation.action.label()),
                false,
                cx,
            ),
            Err(e) => self.message(format!("{e:#}"), true, cx),
        }
    }

    fn resume(&mut self) -> anyhow::Result<()> {
        if let Some(shortcuts) = &mut self.shortcuts {
            self.taken = shortcuts.replace(&self.active)?;
            if let Some(message) = crate::shortcuts::taken_message(&self.active, &self.taken) {
                anyhow::bail!(message);
            }
        }
        Ok(())
    }

    /// The active shortcut for this row could not be registered. A shortcut
    /// edited since then isn't flagged until Save tries it.
    fn is_taken(&self, binding: &Binding) -> bool {
        self.taken
            .iter()
            .any(|(action, _)| *action == binding.action)
            && self
                .active
                .bindings
                .iter()
                .any(|b| b.action == binding.action && b.shortcut == binding.shortcut)
    }

    fn record(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(shortcuts) = &mut self.shortcuts
            && let Err(error) = shortcuts.suspend()
        {
            self.message(format!("{error:#}"), true, cx);
            return;
        }
        self.recording = Some(index);
        self.preview.select(
            self.draft.bindings[index].action,
            self.draft.width_resize.percent,
            Instant::now(),
        );
        self.focus.focus(window);
        self.message(
            "Press a shortcut · Escape cancels · Backspace disables",
            false,
            cx,
        );
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.recording else {
            return;
        };
        cx.stop_propagation();
        let stroke = &event.keystroke;
        if stroke.key == "escape" {
            self.recording = None;
            match self.resume() {
                Ok(()) => self.message("Recording canceled", false, cx),
                Err(e) => self.message(format!("{e:#}"), true, cx),
            }
            return;
        }
        let shortcut = if stroke.key == "backspace" && !stroke.modifiers.modified() {
            String::new()
        } else {
            let mut parts = Vec::new();
            if stroke.modifiers.control {
                parts.push("Ctrl".to_owned());
            }
            if stroke.modifiers.alt {
                parts.push("Alt".to_owned());
            }
            if stroke.modifiers.shift {
                parts.push("Shift".to_owned());
            }
            if stroke.modifiers.platform {
                parts.push("Super".to_owned());
            }
            if parts.is_empty() {
                self.message("Include Ctrl, Alt, Shift, or Super/Command", true, cx);
                return;
            }
            let key = match stroke.key.as_str() {
                "left" => "ArrowLeft",
                "right" => "ArrowRight",
                "up" => "ArrowUp",
                "down" => "ArrowDown",
                "space" => "Space",
                "enter" => "Enter",
                "tab" => "Tab",
                "delete" => "Delete",
                "pageup" => "PageUp",
                "pagedown" => "PageDown",
                "home" => "Home",
                "end" => "End",
                "+" => "Equal",
                "*" if stroke.modifiers.shift => "Digit8",
                "*" => "NumpadMultiply",
                key => key,
            };
            parts.push(key.to_string());
            parts.join("+")
        };
        let mut candidate = self.draft.clone();
        candidate.bindings[index].shortcut = shortcut;
        if let Err(error) = candidate.validate() {
            self.message(format!("{error:#}"), true, cx);
            return;
        }
        self.draft = candidate;
        self.recording = None;
        match self.resume() {
            Ok(()) => self.message(
                "Shortcut updated. Save to activate your changes.",
                false,
                cx,
            ),
            Err(error) => self.message(format!("{error:#}"), true, cx),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        if self.shortcuts.is_none() {
            match Shortcuts::new() {
                Ok(shortcuts) => self.shortcuts = Some(shortcuts),
                Err(error) => {
                    self.message(format!("{error:#}"), true, cx);
                    return;
                }
            }
        }
        let shortcuts = self.shortcuts.as_mut().unwrap();
        let taken = match shortcuts.replace(&self.draft) {
            Ok(taken) => taken,
            Err(error) => {
                let _ = self.resume();
                self.message(format!("{error:#}"), true, cx);
                return;
            }
        };
        if let Err(error) = self.draft.save(&self.path) {
            let rollback = shortcuts
                .replace(&self.active)
                .map(|taken| self.taken = taken);
            self.message(
                format!(
                    "Save failed: {error:#}{}",
                    rollback
                        .err()
                        .map(|e| format!("; restore shortcuts failed: {e:#}"))
                        .unwrap_or_default()
                ),
                true,
                cx,
            );
            return;
        }
        let tray_changed = self.active.tray_icon != self.draft.tray_icon;
        self.active = self.draft.clone();
        let message = crate::shortcuts::taken_message(&self.active, &taken);
        self.taken = taken;
        if let Err(error) = crate::tiling::saved_settings(&self.active, &self.path) {
            self.message(
                format!("Saved, but could not update tiling: {error:#}"),
                true,
                cx,
            );
            return;
        }
        if tray_changed
            && let Err(error) = crate::tray::set_style(&self._tray.icon, self.active.tray_icon)
        {
            self.message(
                format!("Saved, but could not change the tray icon: {error:#}"),
                true,
                cx,
            );
            return;
        }
        if let Err(error) = crate::autostart::apply(self.active.launch_at_login) {
            self.message(format!("Saved, but {error:#}"), true, cx);
            return;
        }
        if let Some(taken) = message {
            self.message(format!("Saved. {taken}"), true, cx);
            return;
        }
        #[cfg(target_os = "linux")]
        if platform::is_gnome_wayland() {
            match std::env::current_exe()
                .and_then(|exe| Ok((exe, std::path::absolute(&self.path)?)))
                .map_err(anyhow::Error::from)
                .and_then(|(exe, path)| crate::gnome::setup(&self.active, &exe, &path))
            {
                Ok(setup) => self.message(format!("Saved. {}", setup.summary()), false, cx),
                Err(error) => self.message(format!("Saved, but {error:#}"), true, cx),
            }
            return;
        }
        self.message(
            if platform::is_wayland() {
                "Saved. This Wayland compositor isn't supported yet: tt-pane runs on GNOME (Wayland) and in X11 sessions."
            } else {
                "Saved. Your shortcuts are active across all applications."
            },
            false,
            cx,
        );
    }

    /// Install the GNOME Shell extension and register GNOME shortcuts.
    fn setup_gnome(&mut self, cx: &mut Context<Self>) {
        #[cfg(not(target_os = "linux"))]
        let _ = cx;
        #[cfg(target_os = "linux")]
        if platform::is_gnome_wayland() {
            let result = std::env::current_exe()
                .and_then(|exe| Ok((exe, std::path::absolute(&self.path)?)))
                .map_err(anyhow::Error::from)
                .and_then(|(exe, path)| crate::gnome::setup(&self.active, &exe, &path));
            match result {
                Ok(setup) => self.message(setup.summary(), false, cx),
                Err(error) => self.message(format!("{error:#}"), true, cx),
            }
        }
    }
}

fn button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x263549))
        .text_color(rgb(0xe8eef7))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x354961)))
        .child(text.to_owned())
}

const TEXT: u32 = 0xe8eef7;
const MUTED: u32 = 0x91a4bb;
const MINT: u32 = 0x6ee7c0;
const PANEL: u32 = 0x131e2c;
const EDGE: u32 = 0x1b2838;
const BAR: u32 = 0x0c1520;
const SAVE: u32 = 0x217b68;

/// The pages of the Settings window, shown as tabs under the header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Shortcuts,
    Snapping,
    Tiling,
    Focus,
    General,
}

impl Tab {
    const ALL: [Self; 5] = [
        Self::Shortcuts,
        Self::Snapping,
        Self::Tiling,
        Self::Focus,
        Self::General,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Shortcuts => "Shortcuts",
            Self::Snapping => "Snapping",
            Self::Tiling => "Tiling",
            Self::Focus => "Focus",
            Self::General => "General",
        }
    }

    /// Whether `draft` has unsaved changes to the settings on this tab.
    fn dirty(self, draft: &Config, active: &Config) -> bool {
        match self {
            Self::Shortcuts => {
                draft.bindings.len() != active.bindings.len()
                    || draft
                        .bindings
                        .iter()
                        .zip(&active.bindings)
                        .any(|(d, a)| d.action != a.action || d.shortcut != a.shortcut)
            }
            Self::Snapping => {
                draft.center_layout != active.center_layout
                    || draft.gaps != active.gaps
                    || draft.width_resize != active.width_resize
                    || draft.growth != active.growth
            }
            Self::Tiling => draft.tiling != active.tiling,
            Self::Focus => draft.focus != active.focus,
            Self::General => {
                draft.launch_at_login != active.launch_at_login
                    || draft.tray_icon != active.tray_icon
            }
        }
    }
}

/// Group headings on the Shortcuts tab, in display order.
const BINDING_GROUPS: [&str; 4] = ["Snap", "Size", "Focus", "Tiling"];

/// The Shortcuts tab group an action is listed under.
fn binding_group(action: Action) -> &'static str {
    if Action::POSITIONS.contains(&action) {
        "Snap"
    } else if action.focuses() {
        "Focus"
    } else if action.is_tiling() {
        "Tiling"
    } else {
        "Size"
    }
}

fn help(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.into())
}

/// One setting: name and help on the left, its control on the right.
fn setting(
    title: impl Into<SharedString>,
    note: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    let note = note.into();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .px_4()
        .py_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().child(title.into()))
                .when(!note.is_empty(), |el| el.child(help(note))),
        )
        .child(div().flex_shrink_0().child(control))
}

/// A setting that only matters when the one above it is on.
fn nested(row: Div) -> Div {
    row.pl(px(36.))
}

/// A titled group of settings on one panel, divided by hairlines.
fn section(title: &str, rows: impl IntoIterator<Item = Div>) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .child(title.to_uppercase()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded_lg()
                .bg(rgb(PANEL))
                .children(
                    rows.into_iter().enumerate().map(|(i, row)| {
                        row.when(i > 0, |el| el.border_t_1().border_color(rgb(EDGE)))
                    }),
                ),
        )
}

fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .p(px(3.))
        .flex()
        .when(on, |el| el.justify_end())
        .rounded_full()
        .bg(rgb(if on { SAVE } else { 0x263549 }))
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .rounded_full()
                .bg(rgb(if on { TEXT } else { MUTED })),
        )
}

fn segments(children: impl IntoIterator<Item = Stateful<Div>>) -> Div {
    div()
        .flex()
        .p(px(2.))
        .rounded_md()
        .bg(rgb(0x172231))
        .children(children)
}

fn segment(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .text_color(rgb(if on { TEXT } else { MUTED }))
        .when(on, |el| el.bg(rgb(0x354961)))
        .when(!on, |el| el.hover(|style| style.text_color(rgb(TEXT))))
        .child(label)
}

fn stepper(less: Stateful<Div>, value: String, more: Stateful<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(less)
        .child(div().w(px(52.)).flex().justify_center().child(value))
        .child(more)
}

fn step_button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    button(id, text).px_3().py_1()
}

/// Help text that isn't tied to one setting.
fn page_note(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .px_4()
        .py_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x263549))
        .text_xs()
        .text_color(rgb(color))
        .child(text.into())
}

impl Settings {
    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            match self.resume() {
                Ok(()) => self.message("Recording canceled", false, cx),
                Err(e) => self.message(format!("{e:#}"), true, cx),
            }
        }
        self.tab = tab;
        cx.notify();
    }

    fn binding_row(&self, index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let binding = &self.draft.bindings[index];
        let active = self.recording == Some(index);
        let action = binding.action;
        let taken = !active && self.is_taken(binding);
        div()
            .id(("binding-row", index))
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    this.hovered_binding = Some(index);
                    if this.preview.action != action {
                        this.preview.select(
                            action,
                            this.draft.width_resize.percent,
                            Instant::now(),
                        );
                    }
                } else if this.hovered_binding == Some(index) {
                    this.hovered_binding = None;
                }
                cx.notify();
            }))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .pl_3()
            .pr_2()
            .py_1()
            .rounded_md()
            .bg(rgb(if active {
                0x203e48
            } else if self.preview.action == action {
                0x1b3040
            } else {
                0x172231
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().child(action.label()))
                    .when(taken, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xf0645a))
                                .child("⚠ In use by another app"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button(
                            ("binding", index),
                            if active {
                                "Recording…"
                            } else if binding.shortcut.is_empty() {
                                "Disabled"
                            } else {
                                &binding.shortcut
                            },
                        )
                        .py_1()
                        .text_sm()
                        .min_w(px(132.))
                        .flex()
                        .justify_center()
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.record(index, window, cx)),
                        ),
                    )
                    .child(div().size(px(30.)).when(
                        !active && !binding.shortcut.is_empty(),
                        |el| {
                            el.child(
                                // Strike-through circle: unassigns this shortcut.
                                div()
                                    .id(("unbind", index))
                                    .size(px(30.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_lg()
                                    // Hidden until the row is hovered; it keeps its
                                    // space so the row never shifts.
                                    .text_color(if self.hovered_binding == Some(index) {
                                        rgb(0xf0645a).into()
                                    } else {
                                        transparent_black()
                                    })
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x3a2128)))
                                    .child("⊘")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.draft.bindings[index].shortcut.clear();
                                        this.message(
                                            format!(
                                                "{} unassigned. Save to activate.",
                                                this.draft.bindings[index].action.label()
                                            ),
                                            false,
                                            cx,
                                        );
                                    })),
                            )
                        },
                    )),
            )
    }

    fn shortcuts_tab(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let selected = self.preview.action;
        let ([preview_x, preview_y, preview_width, preview_height], animating) =
            self.preview.sample(Instant::now());
        if animating {
            window.request_animation_frame();
        }
        let groups = BINDING_GROUPS.map(|name| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(MUTED))
                        .child(name.to_uppercase()),
                )
                .children(
                    (0..self.draft.bindings.len())
                        .filter(|&index| binding_group(self.draft.bindings[index].action) == name)
                        .map(|index| self.binding_row(index, cx))
                        .collect::<Vec<_>>(),
                )
        });
        let notes = div()
            .flex()
            .flex_col()
            .gap_3()
            .when(selected.is_tiling(), |el| el.child(div().text_xs().text_color(rgb(MINT)).child("Experimental tiling action (Windows, macOS, GNOME on Wayland, and X11). Turn on automatic tiling on the Tiling tab or use Toggle experimental tiling. These actions edit the live layout and have no fixed snap position.")))
            .when(selected.adjusts_pair(), |el| el.child(div().text_xs().text_color(rgb(MINT)).child("Move the shared boundary by 25% of the selected window's current height, with a 50/50 stop when returning toward the middle. Blue shows the neighbor. Windows only; matching left and right edges required.")))
            .when(selected.focuses(), |el| el.child(div().text_xs().text_color(rgb(MINT)).child("H: left, J: down, K: up, L: right. Tap H/J/K/L to navigate background windows by default. Inside a centered group, H/L rotate continuously. Hold for 400 ms to enter covered middle windows from an edge, or exit a middle window to a background edge window. Holds are Windows only; centered groups rotate on Windows and GNOME on Wayland.")))
            .when(selected == Action::FillGap, |el| el.child(div().text_xs().text_color(rgb(MINT)).child("Example gap after shrinking the last previewed position. To fill a real gap, shrink a window, select another on the same monitor, then press Ctrl+Alt+. (period).")))
            .when(selected.adjusts_width(), |el| el.child(help("Width change per press is set on the Snapping tab.")))
            .child(help("Hover a shortcut to preview its first position. Green shows the window; the dark area represents the monitor’s usable space."));
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_3()
                    .child(help("HOVER TO PREVIEW · CLICK A SHORTCUT TO RECORD"))
                    .child(
                        div()
                            .id("bindings")
                            .flex()
                            .flex_col()
                            .gap_4()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pr_2()
                            .children(groups),
                    ),
            )
            .child(
                div()
                    .w(px(264.))
                    .flex_shrink_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(PANEL))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Position preview"),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex_shrink_0()
                                    .w(px(PREVIEW_AREA.width as f32))
                                    .h(px(PREVIEW_AREA.height as f32))
                                    .rounded_md()
                                    .overflow_hidden()
                                    .bg(rgb(0x090f19))
                                    .border_1()
                                    .border_color(rgb(0x354961))
                                    .when(selected.adjusts_pair(), |el| {
                                        el.child(
                                            div()
                                                .absolute()
                                                .left(px(preview_x))
                                                .top(px(0.))
                                                .w(px(preview_width))
                                                .h(px(preview_y.max(0.)))
                                                .bg(rgb(0x355b91))
                                                .border_1()
                                                .border_color(rgb(0x8bb5ef)),
                                        )
                                    })
                                    .child(
                                        div()
                                            .absolute()
                                            .left(px(preview_x))
                                            .top(px(preview_y))
                                            .w(px(preview_width))
                                            .h(px(preview_height))
                                            .rounded_sm()
                                            .bg(rgb(0x258c78))
                                            .border_1()
                                            .border_color(rgb(MINT)),
                                    ),
                            )
                            .child(div().text_sm().child(selected.label())),
                    )
                    .child(
                        div()
                            .id("action-notes")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(PANEL))
                            .child(notes),
                    ),
            )
    }

    fn snapping_tab(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let d = &self.draft;
        vec![
            section(
                "Center windows",
                [setting(
                    "Layout",
                    "Offset spreads background center windows out. Stack keeps them centered; H/L brings the next window forward. Sizing stays the same.",
                    segments(
                        [
                            (CenterLayout::Offset, "center-offset", "Offset"),
                            (CenterLayout::Stack, "center-stack", "Stack"),
                        ]
                        .map(|(layout, id, label)| {
                            segment(id, label, d.center_layout == layout).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if this.draft.center_layout != layout {
                                        this.draft.center_layout = layout;
                                        this.message(
                                            "Center layout updated. Save to activate.",
                                            false,
                                            cx,
                                        );
                                    }
                                },
                            ))
                        }),
                    ),
                )],
            ),
            section(
                "Snapped window gaps",
                [
                    setting(
                        "Inner gap per window",
                        "Default 5 on each inner edge, so 10 between tiles. 0 turns spacing off.",
                        stepper(
                            step_button("gap-inner-less", "−").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.gaps.inner = this.draft.gaps.inner.saturating_sub(1);
                                    this.message(
                                        "Window gaps updated. Save to activate.",
                                        false,
                                        cx,
                                    );
                                },
                            )),
                            d.gaps.inner.to_string(),
                            step_button("gap-inner-more", "+").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.gaps.inner = (this.draft.gaps.inner + 1).min(100);
                                    this.message(
                                        "Window gaps updated. Save to activate.",
                                        false,
                                        cx,
                                    );
                                },
                            )),
                        ),
                    ),
                    setting(
                        "Desktop edge gap",
                        "Default 20, range 0–100. Applies on the next snap; center windows are unchanged.",
                        stepper(
                            step_button("gap-outer-less", "−").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.gaps.outer = this.draft.gaps.outer.saturating_sub(1);
                                    this.message(
                                        "Window gaps updated. Save to activate.",
                                        false,
                                        cx,
                                    );
                                },
                            )),
                            d.gaps.outer.to_string(),
                            step_button("gap-outer-more", "+").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.gaps.outer = (this.draft.gaps.outer + 1).min(100);
                                    this.message(
                                        "Window gaps updated. Save to activate.",
                                        false,
                                        cx,
                                    );
                                },
                            )),
                        ),
                    ),
                ],
            ),
            section(
                "Width adjustment",
                [setting(
                    "Width change per press",
                    "Shrink and Expand width keep the height and the left edge, right edge, or center fixed. Repeats keep resizing; expansion stops at the work-area edge.",
                    stepper(
                        step_button("width-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.width_resize.percent =
                                this.draft.width_resize.percent.saturating_sub(1).max(1);
                            this.preview.select(
                                this.preview.action,
                                this.draft.width_resize.percent,
                                Instant::now(),
                            );
                            this.message("Width adjustment updated. Save to activate.", false, cx);
                        })),
                        format!("{}%", d.width_resize.percent),
                        step_button("width-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.width_resize.percent =
                                (this.draft.width_resize.percent + 1).min(99);
                            this.preview.select(
                                this.preview.action,
                                this.draft.width_resize.percent,
                                Instant::now(),
                            );
                            this.message("Width adjustment updated. Save to activate.", false, cx);
                        })),
                    ),
                )],
            ),
            section(
                "Repeat shortcut to grow",
                [
                    setting(
                        "Growth per press",
                        "The first press places the window. Repeats multiply its size, keeping the corner or edge anchored. Center starts at 50% and grows equally in every direction.",
                        stepper(
                            step_button("growth-less", "−").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.growth.percent =
                                        this.draft.growth.percent.saturating_sub(1).max(1);
                                    this.message("Growth updated. Save to activate.", false, cx);
                                },
                            )),
                            format!("{}%", d.growth.percent),
                            step_button("growth-more", "+").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.growth.percent =
                                        (this.draft.growth.percent + 1).min(100);
                                    this.message("Growth updated. Save to activate.", false, cx);
                                },
                            )),
                        ),
                    ),
                    setting(
                        "Steps before reset",
                        "Growth stops at the work area. 0 turns it off.",
                        stepper(
                            step_button("steps-less", "−").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.growth.steps =
                                        this.draft.growth.steps.saturating_sub(1);
                                    this.message("Growth updated. Save to activate.", false, cx);
                                },
                            )),
                            if d.growth.steps == 0 {
                                "Off".to_owned()
                            } else {
                                d.growth.steps.to_string()
                            },
                            step_button("steps-more", "+").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft.growth.steps = (this.draft.growth.steps + 1).min(20);
                                    this.message("Growth updated. Save to activate.", false, cx);
                                },
                            )),
                        ),
                    ),
                ],
            ),
            page_note(
                "Positions use the active window’s monitor and its available work area. A window spanning displays uses the monitor with the largest overlap.",
                MUTED,
            ),
        ]
    }

    fn tiling_tab(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let tiling = &self.draft.tiling;
        let mut page = Vec::new();
        // Tiling runs wherever tt-pane can inspect windows: Windows, macOS,
        // GNOME on Wayland (through the Shell extension) and X11.
        if platform::tiling_supported() {
            page.push(section("Automatic tiling", [
                setting(
                    "Tile windows automatically (experimental)",
                    "Dwindle tiles restored windows on the current desktop. Moving a tile floats it; a snapped window keeps its space and the tiles fill the rest.",
                    switch("tiling-enabled", tiling.enabled).on_click(cx.listener(|this, _, _, cx| {
                        this.draft.tiling.enabled = !this.draft.tiling.enabled;
                        this.message("Automatic tiling updated. Save to activate.", false, cx);
                    })),
                ),
            ]));
        }
        page.push(if platform::tiling_supported() {
            page_note("Tiling shortcuts (swap, split, float, toggle tiling) are on the Shortcuts tab under Tiling.", MINT)
        } else {
            page_note("Automatic tiling is available on Windows, macOS, GNOME on Wayland, and X11.", MUTED)
        });
        page
    }

    fn focus_tab(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let focus = &self.draft.focus;
        let mut page = Vec::new();
        if cfg!(windows) {
            page.push(section(
                "Focus glow",
                [
                    setting(
                        "Highlight the focused window",
                        "Briefly highlight the target after moving a window or using H/J/K/L.",
                        switch("focus-glow", focus.glow_enabled).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.draft.focus.glow_enabled = !this.draft.focus.glow_enabled;
                                this.message("Focus glow updated. Save to activate.", false, cx);
                            },
                        )),
                    ),
                    nested(setting(
                        "Glow color",
                        "Default: neutral off-white.",
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(18.))
                                    .rounded_sm()
                                    .bg(rgb(focus.glow_rgb().unwrap_or(0xf2f0e8))),
                            )
                            .child(
                                button("glow-color", &focus.glow_color).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        #[cfg(windows)]
                                        {
                                            let initial =
                                                this.draft.focus.glow_rgb().unwrap_or(0xf2f0e8);
                                            cx.spawn(async move |this, cx| {
                                                // ChooseColorW pumps Windows messages. Run it after the
                                                // click listener releases GPUI's App/entity borrows so
                                                // activation observers and foreground tasks can re-enter.
                                                let color = platform::choose_glow_color(initial);
                                                if let Some(color) = color {
                                                    let _ = this.update(cx, |this, cx| {
                                                        this.draft.focus.glow_color =
                                                            format!("#{color:06X}");
                                                        this.message(
                                                            "Glow color updated. Save to activate.",
                                                            false,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            })
                                            .detach();
                                        }
                                        let _ = (this, cx);
                                    },
                                )),
                            ),
                    )),
                ],
            ));
        }
        page.push(section("H/J/K/L navigation", [setting(
            "Include middle windows",
            "Navigate into windows that sit between the screen edges, not only the ones at each edge.",
            switch("include-middle", focus.include_middle_windows).on_click(cx.listener(|this, _, _, cx| {
                this.draft.focus.include_middle_windows = !this.draft.focus.include_middle_windows;
                this.message("Window focus updated. Save to activate.", false, cx);
            })),
        )]));
        page.push(section("Middle-window visibility", [
            setting(
                "Filter middle windows by visibility",
                "Applies when middle windows are included. Off includes covered windows. Minimized windows and other virtual desktops stay excluded.",
                switch("visibility-filter", focus.visibility_filter_enabled).on_click(cx.listener(|this, _, _, cx| {
                    this.draft.focus.visibility_filter_enabled = !this.draft.focus.visibility_filter_enabled;
                    this.message("Window focus updated. Save to activate.", false, cx);
                })),
            ),
            nested(setting(
                "Minimum visible",
                "A middle window must show more than this to be included. Default: 10%.",
                stepper(
                    step_button("visibility-less", "−").on_click(cx.listener(|this, _, _, cx| {
                        this.draft.focus.minimum_visible_percent = this.draft.focus.minimum_visible_percent.saturating_sub(1);
                        this.message("Window focus updated. Save to activate.", false, cx);
                    })),
                    format!("{}%", focus.minimum_visible_percent),
                    step_button("visibility-more", "+").on_click(cx.listener(|this, _, _, cx| {
                        this.draft.focus.minimum_visible_percent = (this.draft.focus.minimum_visible_percent + 1).min(99);
                        this.message("Window focus updated. Save to activate.", false, cx);
                    })),
                ),
            )),
        ]));
        page.push(page_note("Hold H/L for 400 ms to enter covered middle windows from an edge, or to return from a middle window to the background.", MUTED));
        page
    }

    fn general_tab(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let mut page = vec![section(
            "Startup",
            [setting(
                "Launch at login",
                "Start tt-pane in the tray when you sign in.",
                switch("launch-at-login", self.draft.launch_at_login).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.draft.launch_at_login = !this.draft.launch_at_login;
                        this.message("Launch at login updated. Save to activate.", false, cx);
                    },
                )),
            )],
        )];
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            page.push(section("Appearance", [setting(
                if cfg!(target_os = "macos") { "Menu bar icon" } else { "Tray icon" },
                if cfg!(target_os = "macos") {
                    "Monochrome follows the menu bar's light or dark appearance. Color shows the full app icon."
                } else {
                    "Monochrome draws a white silhouette for dark panels. Color shows the full app icon."
                },
                segments([
                    (TrayIconStyle::Color, "tray-color", "Color"),
                    (TrayIconStyle::Monochrome, "tray-monochrome", "Monochrome"),
                ]
                .map(|(style, id, label)| {
                    segment(id, label, self.draft.tray_icon == style).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if this.draft.tray_icon != style {
                                this.draft.tray_icon = style;
                                this.message("Tray icon updated. Save to activate.", false, cx);
                            }
                        },
                    ))
                })),
            )]));
        }
        let pending = self.update.pending.is_some();
        page.push(section(
            "Updates",
            [setting(
                format!("Version {}", env!("CARGO_PKG_VERSION")),
                self.update.message.clone(),
                div()
                    .flex()
                    .gap_2()
                    .when(!pending, |el| {
                        el.child(
                            button("check-update", "Check for updates")
                                .on_click(cx.listener(|this, _, _, cx| this.check_update(cx))),
                        )
                    })
                    .when(!pending && self.update.release.is_some(), |el| {
                        el.child(
                            button("install-update", "Update and restart")
                                .bg(rgb(SAVE))
                                .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                        )
                    }),
            )],
        ));
        if platform::is_gnome_wayland() {
            page.push(section("GNOME", [setting(
                "GNOME Shell extension and shortcuts",
                "Saving registers your shortcuts as GNOME keyboard shortcuts. The first time, log out and back in so GNOME loads the tt-pane extension.",
                button("setup-gnome", "Set up GNOME").on_click(cx.listener(|this, _, _, cx| this.setup_gnome(cx))),
            )]));
        }
        page.push(page_note(
            if platform::is_wayland() && !platform::is_gnome_wayland() {
                "This Wayland compositor isn't supported yet. tt-pane runs on GNOME (Wayland) and in X11 sessions."
            } else {
                "Closing settings keeps tt-pane running in the tray. Use the tray menu to quit."
            },
            MUTED,
        ));
        page
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.update.installing {
            return div()
                .size_full()
                .bg(rgb(0x0e1724))
                .text_color(rgb(TEXT))
                .flex()
                .flex_col()
                .p_6()
                .gap_4()
                .child(div().text_2xl().child("Updating tt-pane"))
                .child(self.update.message.clone())
                .child(crate::update::RESTART_NOTE)
                .into_any_element();
        }
        let tabs = Tab::ALL.map(|tab| {
            let selected = self.tab == tab;
            div()
                .id(("tab", tab as usize))
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .pt_2()
                .pb(px(10.))
                .border_b_2()
                .border_color(if selected {
                    rgb(MINT).into()
                } else {
                    transparent_black()
                })
                .text_sm()
                .text_color(rgb(if selected { TEXT } else { MUTED }))
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(TEXT)))
                .child(tab.label())
                .when(tab.dirty(&self.draft, &self.active), |el| {
                    el.child(div().size(px(6.)).rounded_full().bg(rgb(MINT)))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.select_tab(tab, cx)))
        });
        let page = match self.tab {
            Tab::Shortcuts => self.shortcuts_tab(window, cx).px_6().py_4(),
            tab => {
                let sections = match tab {
                    Tab::Snapping => self.snapping_tab(cx),
                    Tab::Tiling => self.tiling_tab(cx),
                    Tab::Focus => self.focus_tab(cx),
                    _ => self.general_tab(cx),
                };
                div().flex().flex_col().flex_1().min_h_0().child(
                    div()
                        .id(("page", tab as usize))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_6()
                        .py_4()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(680.))
                                .flex()
                                .flex_col()
                                .gap_5()
                                .children(sections),
                        ),
                )
            }
        };
        let content = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(0x0e1724))
            .text_color(rgb(TEXT))
            .font_family("Segoe UI")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .px_6()
                    .pt_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("tt-pane"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("Your windows. A keystroke away."),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MINT))
                            .child(platform::description()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .px_6()
                    .pt_3()
                    .border_b_1()
                    .border_color(rgb(EDGE))
                    .children(tabs),
            )
            .child(page)
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_6()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(EDGE))
                    .bg(rgb(BAR))
                    .child(button("reset", "Reset defaults").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.recording = None;
                            let result = this.resume();
                            this.draft = Config::default();
                            this.preview.select(
                                this.preview.action,
                                this.draft.width_resize.percent,
                                Instant::now(),
                            );
                            this.message(
                                result.err().map(|e| format!("{e:#}")).unwrap_or_else(|| {
                                    "Defaults restored. Save to activate.".into()
                                }),
                                false,
                                cx,
                            );
                        },
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(if self.error { 0xffa59b } else { MUTED }))
                            .child(self.status.clone()),
                    )
                    .child(
                        button("save", "Save changes")
                            .bg(rgb(SAVE))
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            );
        window_frame(content, window)
    }
}

const WINDOW_TITLE: &str = "tt-pane — Settings";
/// Width of the transparent band around a client-decorated window that holds
/// its shadow and resize handles.
const FRAME_INSET: Pixels = px(10.);
const FRAME_RADIUS: Pixels = px(10.);

/// Wrap `content` in a title bar, border, shadow and resize edges when the
/// compositor leaves decorations to the app (GNOME on Wayland has no
/// server-side decorations). Elsewhere the system draws the frame.
fn window_frame(content: Div, window: &mut Window) -> AnyElement {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return div()
            .size_full()
            .flex()
            .flex_col()
            .child(content)
            .into_any_element();
    };
    let inset = if tiling.is_tiled() {
        px(0.)
    } else {
        FRAME_INSET
    };
    window.set_client_inset(inset);
    let controls = window.window_controls();
    let control = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .text_color(rgb(0xc5d1e0))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x263549)))
            // Keep the press from starting a window move in the title bar.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(label)
    };
    let title_bar = div()
        .id("title-bar")
        .flex_shrink_0()
        .h(px(40.))
        .pl_4()
        .pr_2()
        .flex()
        .items_center()
        .justify_between()
        .bg(rgb(0x0b131e))
        .border_b_1()
        .border_color(rgb(0x1b2838))
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position)
        })
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x91a4bb))
                .child(WINDOW_TITLE),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .when(controls.minimize, |el| {
                    el.child(
                        control("window-minimize", "–")
                            .on_click(|_, window, _| window.minimize_window()),
                    )
                })
                .when(controls.maximize, |el| {
                    el.child(
                        control("window-maximize", if tiling.is_tiled() { "❐" } else { "□" })
                            .on_click(|_, window, _| window.zoom_window()),
                    )
                })
                .child(
                    control("window-close", "✕")
                        .hover(|style| style.bg(rgb(0x8a2f2a)))
                        .on_click(|_, window, _| window.remove_window()),
                ),
        );
    div()
        .id("window-backdrop")
        .size_full()
        .bg(transparent_black())
        .when(!tiling.top, |el| el.pt(inset))
        .when(!tiling.bottom, |el| el.pb(inset))
        .when(!tiling.left, |el| el.pl(inset))
        .when(!tiling.right, |el| el.pr(inset))
        .child(
            canvas(
                |_, window, _| {
                    window.insert_hitbox(
                        Bounds::new(
                            point(px(0.), px(0.)),
                            window.window_bounds().get_bounds().size,
                        ),
                        HitboxBehavior::Normal,
                    )
                },
                move |_, hitbox, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(window.mouse_position(), inset, size) {
                        window.set_cursor_style(
                            match edge {
                                ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                ResizeEdge::Left | ResizeEdge::Right => {
                                    CursorStyle::ResizeLeftRight
                                }
                                ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                    CursorStyle::ResizeUpLeftDownRight
                                }
                                ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                    CursorStyle::ResizeUpRightDownLeft
                                }
                            },
                            &hitbox,
                        );
                    }
                },
            )
            .size_full()
            .absolute(),
        )
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(event.position, inset, size) {
                window.start_window_resize(edge);
            }
        })
        .child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .cursor(CursorStyle::Arrow)
                .bg(rgb(0x0e1724))
                .border_color(rgb(0x354961))
                .when(!tiling.top, |el| el.border_t_1())
                .when(!tiling.bottom, |el| el.border_b_1())
                .when(!tiling.left, |el| el.border_l_1())
                .when(!tiling.right, |el| el.border_r_1())
                .when(!(tiling.top || tiling.left), |el| {
                    el.rounded_tl(FRAME_RADIUS)
                })
                .when(!(tiling.top || tiling.right), |el| {
                    el.rounded_tr(FRAME_RADIUS)
                })
                .when(!(tiling.bottom || tiling.left), |el| {
                    el.rounded_bl(FRAME_RADIUS)
                })
                .when(!(tiling.bottom || tiling.right), |el| {
                    el.rounded_br(FRAME_RADIUS)
                })
                .when(!tiling.is_tiled(), |el| {
                    el.shadow(vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.4),
                        blur_radius: FRAME_INSET / 2.,
                        spread_radius: px(0.),
                        offset: point(px(0.), px(0.)),
                    }])
                })
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(title_bar)
                .child(content),
        )
        .into_any_element()
}

/// The edge or corner under `pos` within the frame's `inset` band.
fn resize_edge(pos: Point<Pixels>, inset: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let top = pos.y < inset;
    let bottom = pos.y > size.height - inset;
    let left = pos.x < inset;
    let right = pos.x > size.width - inset;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

fn show_settings(settings: &Entity<Settings>, cx: &mut App) -> anyhow::Result<()> {
    if let Some(handle) = settings.read(cx).window
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        cx.activate(true);
        return Ok(());
    }
    let bounds = Bounds::centered(None, size(px(900.), px(820.)), cx);
    let view = settings.clone();
    let window = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(WINDOW_TITLE.into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(600.))),
            app_id: Some("dev.ttpane.TtPane".into()),
            // GPUI defaults to asking for server-side decorations and treats the
            // window as framed even when the compositor can't draw them (GNOME
            // has no xdg-decoration), leaving it without a title bar. On
            // Wayland draw our own frame instead; see `window_frame`.
            window_decorations: platform::is_wayland().then_some(WindowDecorations::Client),
            ..Default::default()
        },
        move |window, cx| {
            view.update(cx, |this, cx| {
                #[cfg(target_os = "windows")]
                if let Err(error) = crate::platform::set_window_icon(window) {
                    this.message(format!("Could not set window icon: {error:#}"), true, cx);
                }
                this.focus.focus(window);
                cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() && this.recording.take().is_some() {
                        match this.resume() {
                            Ok(()) => this.message(
                                "Recording canceled when settings lost focus",
                                false,
                                cx,
                            ),
                            Err(e) => this.message(format!("{e:#}"), true, cx),
                        }
                    }
                })
                .detach();
            });
            view
        },
    )?;
    settings.update(cx, |this, _| this.window = Some(window));
    cx.activate(true);
    Ok(())
}

pub fn run(mut config: Config, path: PathBuf, open_settings: bool) -> anyhow::Result<()> {
    // Without a display GPUI runs headless: the tray (D-Bus) still appears,
    // but Settings can never open, so refuse to start instead.
    #[cfg(target_os = "linux")]
    anyhow::ensure!(
        ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|var| std::env::var_os(var).is_some_and(|v| !v.is_empty())),
        "No graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set. Start tt-pane from \
         the desktop, or set WAYLAND_DISPLAY=wayland-0 (or DISPLAY=:0) when launching it from \
         a terminal such as SSH or tmux."
    );
    // Terminal Ctrl+C quits through the tray loop below, so the exit code is 0.
    #[cfg(windows)]
    platform::quit_on_ctrl_c();
    // A persistent entity owns the shortcuts and tray independently of any window.
    let failure = std::rc::Rc::new(std::cell::RefCell::new(None));
    let result = failure.clone();
    // Launching tt-pane.app again from Finder, Launchpad or Spotlight while it
    // runs reaches the running instance as a reopen; show its settings, since
    // a menu-bar app otherwise gives no sign it heard.
    let reopened: std::rc::Rc<std::cell::RefCell<Option<Entity<Settings>>>> = Default::default();
    let app = Application::new();
    let on_reopen = reopened.clone();
    app.on_reopen(move |cx| {
        let settings = on_reopen.borrow().clone();
        if let Some(settings) = settings
            && let Err(e) = show_settings(&settings, cx)
        {
            settings.update(cx, |this, cx| this.message(format!("{e:#}"), true, cx));
        }
    });
    app.run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
            let mtm = objc2::MainThreadMarker::new().expect("GPUI main thread");
            NSApplication::sharedApplication(mtm)
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
        let tray = match crate::tray::create(config.tray_icon) {
            Ok(tray) => tray,
            Err(e) => {
                *failure.borrow_mut() = Some(e);
                cx.quit();
                return;
            }
        };
        for action in Action::ALL {
            if !config.bindings.iter().any(|b| b.action == action) {
                config.bindings.push(Binding {
                    action,
                    shortcut: String::new(),
                });
            }
        }
        let mut status = format!("Configuration: {}", path.display());
        let mut error = false;
        // Wayland desktops find a window's icon through its desktop entry.
        #[cfg(target_os = "linux")]
        if let Err(e) = crate::launcher::ensure() {
            status = format!("{e:#}");
            error = true;
        }
        // Keep the sign-in entry in step with the setting and the current
        // executable. Debug builds skip registering so `cargo run` never
        // installs a login item pointing into target/.
        if !(config.launch_at_login && cfg!(debug_assertions))
            && let Err(e) = crate::autostart::apply(config.launch_at_login)
        {
            status = format!("{e:#}");
            error = true;
        }
        // GNOME runs tt-pane's shortcuts itself; keep them pointed at this
        // executable. Debug builds skip it, as with autostart above.
        #[cfg(target_os = "linux")]
        if platform::is_gnome_wayland()
            && !cfg!(debug_assertions)
            && let Err(e) = std::env::current_exe()
                .and_then(|exe| Ok((exe, std::path::absolute(&path)?)))
                .map_err(anyhow::Error::from)
                .and_then(|(exe, path)| crate::gnome::setup(&config, &exe, &path))
        {
            status = format!("{e:#}");
            error = true;
        }
        let mut taken = Vec::new();
        let shortcuts = match Shortcuts::new() {
            Ok(mut shortcuts) => {
                match shortcuts.replace(&config) {
                    Ok(keys) => {
                        if let Some(message) = crate::shortcuts::taken_message(&config, &keys) {
                            status = message;
                            error = true;
                        }
                        taken = keys;
                    }
                    Err(e) => {
                        status = format!("{e:#}");
                        error = true;
                    }
                }
                Some(shortcuts)
            }
            Err(e) => {
                status = format!("{e:#}");
                error = true;
                None
            }
        };
        // Ask macOS for Accessibility access up front (its dialog adds tt-pane
        // to System Settings), and open settings to explain why, rather than
        // failing silently on the first shortcut.
        #[cfg(target_os = "macos")]
        let open_settings = if !error && !crate::platform::accessibility_trusted(true) {
            status = "tt-pane needs Accessibility access to move windows. Turn on tt-pane in \
                System Settings → Privacy & Security → Accessibility; shortcuts work as soon as \
                it is on."
                .into();
            error = true;
            true
        } else {
            open_settings
        };
        if error {
            let _ = tray.icon.set_tooltip(Some(format!("tt-pane — {status}")));
        }
        let settings = cx.new(|cx| Settings {
            draft: config.clone(),
            active: config,
            path,
            shortcuts,
            taken,
            recording: None,
            focus: cx.focus_handle(),
            status,
            error,
            preview: PositionPreview::new(Action::Left),
            tab: Tab::Shortcuts,
            hovered_binding: None,
            window: None,
            _tray: tray,
            update: UpdateState::default(),
        });
        *reopened.borrow_mut() = Some(settings.clone());
        let on_close = settings.clone();
        cx.on_window_closed(move |cx| {
            if cx.windows().is_empty() {
                on_close.update(cx, |this, cx| {
                    this.window = None;
                    this.recording = None;
                    if let Err(e) = this.resume() {
                        this.message(format!("{e:#}"), true, cx);
                    }
                });
            }
        })
        .detach();
        if open_settings && let Err(e) = show_settings(&settings, cx) {
            settings.update(cx, |this, cx| this.message(format!("{e:#}"), true, cx));
        }
        cx.spawn(async move |cx| {
            let mut group_observed = std::time::Instant::now();
            // Tiling and centered groups need window inspection: native on Windows
            // and macOS, the Shell extension on GNOME. Other Linux sessions have none.
            let observe_groups = platform::tiling_supported();
            loop {
                smol::Timer::after(Duration::from_millis(16)).await;
                if cx
                    .update(|cx| {
                        #[cfg(windows)]
                        if platform::ctrl_c_requested() {
                            cx.quit();
                            return;
                        }
                        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                            if event.id.0 == crate::tray::UPDATE {
                                if let Err(e) = show_settings(&settings, cx) {
                                    settings.update(cx, |this, cx| {
                                        this.message(format!("{e:#}"), true, cx)
                                    });
                                    continue;
                                }
                                settings.update(cx, |this, cx| {
                                    this.tab = Tab::General;
                                    if this.update.release.is_some() {
                                        this.install_update(cx);
                                    } else {
                                        this.check_update(cx);
                                    }
                                });
                            }
                            if event.id.0 == crate::tray::QUIT {
                                cx.quit();
                                return;
                            }
                            if event.id.0 == crate::tray::SETTINGS
                                && let Err(e) = show_settings(&settings, cx)
                            {
                                settings.update(cx, |this, cx| {
                                    this.message(format!("{e:#}"), true, cx)
                                });
                            }
                        }
                        settings.update(cx, |this, cx| {
                            this.poll_update(cx);
                            if observe_groups
                                && group_observed.elapsed() >= Duration::from_millis(100)
                            {
                                group_observed = std::time::Instant::now();
                                if let Err(e) = crate::tiling::observe(&this.active, &this.path) {
                                    this.message(format!("Tiling: {e:#}"), true, cx);
                                }
                                #[cfg(any(windows, target_os = "linux"))]
                                if let Err(e) =
                                    crate::centered::observe(&this.path, this.active.center_layout)
                                {
                                    this.message(format!("{e:#}"), true, cx);
                                }
                            }
                            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                                if this.recording.is_none()
                                    && let Some(invocation) =
                                        this.shortcuts.as_mut().and_then(|s| {
                                            s.event(event.id, event.state, Instant::now())
                                        })
                                {
                                    this.invoke(invocation, cx);
                                }
                            }
                            if this.recording.is_none()
                                && let Some(invocation) =
                                    this.shortcuts.as_mut().and_then(|s| s.poll(Instant::now()))
                            {
                                this.invoke(invocation, cx);
                            }
                        });
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    });
    match result.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod preview_tests {
    use super::{Action, Instant, PositionPreview};

    #[test]
    fn tiling_hover_preserves_snap_anchor_for_width_and_gap_previews() {
        let now = Instant::now();
        for placement in Action::POSITIONS {
            for tiling in Action::TILING {
                for next in [Action::ShrinkWidth, Action::ExpandWidth, Action::FillGap] {
                    let mut preview = PositionPreview::new(placement);
                    preview.select(tiling, 25, now);
                    preview.select(next, 25, now);
                    let mut expected = PositionPreview::new(placement);
                    expected.select(next, 25, now);
                    assert_eq!(preview.placement, placement);
                    assert_eq!(preview.to, expected.to);
                }
            }
        }
    }
}

#[cfg(test)]
mod tab_tests {
    use super::{Config, Tab};

    #[test]
    fn each_change_marks_only_its_own_tab() {
        let active = Config::default();
        type Change = fn(&mut Config);
        let changes: [(Tab, Change); 5] = [
            (Tab::Shortcuts, |c| c.bindings[0].shortcut.clear()),
            (Tab::Snapping, |c| c.gaps.outer += 1),
            (Tab::Tiling, |c| c.tiling.enabled = !c.tiling.enabled),
            (Tab::Focus, |c| {
                c.focus.include_middle_windows = !c.focus.include_middle_windows
            }),
            (Tab::General, |c| c.launch_at_login = !c.launch_at_login),
        ];
        for (changed, change) in changes {
            let mut draft = active.clone();
            change(&mut draft);
            for tab in Tab::ALL {
                assert_eq!(
                    tab.dirty(&draft, &active),
                    tab == changed,
                    "{changed:?} → {tab:?}"
                );
            }
        }
    }
}
