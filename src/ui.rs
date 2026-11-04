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
        if !action.focuses()
            && !action.adjusts_width()
            && !action.adjusts_pair()
            && action != Action::FillGap
        {
            self.placement = action;
        }
        let target = if action.focuses() {
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
    recording: Option<usize>,
    focus: FocusHandle,
    status: String,
    error: bool,
    preview: PositionPreview,
    advanced_expanded: bool,
    window: Option<WindowHandle<Settings>>,
    _tray: crate::tray::Tray,
    #[cfg(windows)]
    update: UpdateState,
}

#[cfg(windows)]
#[derive(Default)]
struct UpdateState {
    schedule: crate::update::CheckSchedule,
    installing: bool,
    pending: Option<std::sync::mpsc::Receiver<anyhow::Result<UpdateResult>>>,
    release: Option<crate::update::Release>,
    message: String,
}

#[cfg(windows)]
enum UpdateResult {
    Checked(Option<crate::update::Release>),
    Restart,
}

impl Settings {
    #[cfg(windows)]
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

    #[cfg(windows)]
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

    #[cfg(windows)]
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
            shortcuts.replace(&self.active)?;
        }
        Ok(())
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
        if let Err(error) = shortcuts.replace(&self.draft) {
            let _ = self.resume();
            self.message(format!("{error:#}"), true, cx);
            return;
        }
        if let Err(error) = self.draft.save(&self.path) {
            let rollback = shortcuts.replace(&self.active);
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

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(windows)]
        if self.update.installing {
            return div().size_full().bg(rgb(0x0e1724)).text_color(rgb(0xe8eef7))
                .flex().flex_col().p_6().gap_4()
                .child(div().text_2xl().child("Updating tt-pane"))
                .child(self.update.message.clone())
                .child("Your saved settings will be kept. Window shortcuts remain available while downloading.")
                .into_any_element();
        }
        let selected = self.preview.action;
        let ([preview_x, preview_y, preview_width, preview_height], animating) =
            self.preview.sample(Instant::now());
        if animating {
            window.request_animation_frame();
        }
        let rows = self
            .draft
            .bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                let active = self.recording == Some(index);
                let action = binding.action;
                div()
                    .id(("binding-row", index))
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered && this.preview.action != action {
                            this.preview.select(
                                action,
                                this.draft.width_resize.percent,
                                Instant::now(),
                            );
                            cx.notify();
                        }
                    }))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(if active {
                        0x203e48
                    } else if selected == action {
                        0x1b3040
                    } else {
                        0x172231
                    }))
                    .child(div().text_sm().child(binding.action.label()))
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
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.record(index, window, cx)),
                        ),
                    )
            })
            .collect::<Vec<_>>();
        let content = div().track_focus(&self.focus).on_key_down(cx.listener(Self::key_down))
            .flex_1().min_h_0().w_full().bg(rgb(0x0e1724)).text_color(rgb(0xe8eef7)).font_family("Segoe UI")
            .flex().flex_col().p_6().gap_4()
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap_1().child(div().text_3xl().font_weight(FontWeight::BOLD).child("tt-pane"))
                    .child(div().text_sm().text_color(rgb(0x91a4bb)).child("Your windows. A keystroke away.")))
                .child(div().text_xs().text_color(rgb(0x6ee7c0)).child(platform::description())))
            .child(div().flex().flex_1().min_h_0().gap_6()
                .child(div().id("bindings").flex().flex_col().gap_2().flex_1().overflow_y_scroll()
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).mb_2().child("HOVER TO PREVIEW · CLICK A SHORTCUT TO RECORD"))
                    .children(rows))
                .child(div().id("placement-options").w(px(264.)).flex_shrink_0().min_h_0().overflow_y_scroll().flex().flex_col().gap_4().p_4().rounded_lg().bg(rgb(0x131e2c))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Position preview"))
                    .child(div().relative().flex_shrink_0().w(px(PREVIEW_AREA.width as f32)).h(px(PREVIEW_AREA.height as f32)).rounded_md().overflow_hidden().bg(rgb(0x090f19)).border_1().border_color(rgb(0x354961))
                        .when(selected.adjusts_pair(), |el| el.child(div().absolute().left(px(preview_x)).top(px(0.)).w(px(preview_width)).h(px(preview_y.max(0.))).bg(rgb(0x355b91)).border_1().border_color(rgb(0x8bb5ef))))
                        .child(div().absolute().left(px(preview_x)).top(px(preview_y))
                            .w(px(preview_width)).h(px(preview_height)).rounded_sm()
                            .bg(rgb(0x258c78)).border_1().border_color(rgb(0x6ee7c0))))
                    .child(div().text_sm().child(selected.label()))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Hover a keybinding to preview its first position. Green shows the window; the dark area represents the monitor’s usable space."))
                    .when(selected.adjusts_pair(), |el| el.child(div().text_xs().text_color(rgb(0x6ee7c0)).child("Move the shared boundary by 25% of the selected window's current height, with a 50/50 stop when returning toward the middle. Blue shows the neighbor. Windows only; matching left and right edges required.")))
                    .when(selected.focuses(), |el| el.child(div().text_xs().text_color(rgb(0x6ee7c0)).child("H: left, J: down, K: up, L: right. Tap H/J/K/L to navigate background windows by default. Inside a centered group, H/L rotate continuously. Hold for 400 ms to enter covered middle windows from an edge, or exit a middle window to a background edge window. Holds are Windows only; centered groups rotate on Windows and GNOME on Wayland.")))
                    .when(selected == Action::FillGap, |el| el.child(div().text_xs().text_color(rgb(0x6ee7c0)).child("Example gap after shrinking the last previewed position. To fill a real gap, shrink a window, select another on the same monitor, then press Ctrl+Alt+Shift+8 (*).")))
                    .child(div().text_sm().text_color(rgb(0x91a4bb)).child("Positions use the active window’s monitor and its available work area. A window spanning displays uses the monitor with the largest overlap."))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("General"))
                    .child(div().text_sm().child("Launch at login"))
                    .child(button("launch-at-login", if self.draft.launch_at_login { "On" } else { "Off" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.launch_at_login = !this.draft.launch_at_login;
                            this.message("Launch at login updated. Save to activate.", false, cx);
                        })))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Start tt-pane in the tray when you sign in."))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Center windows"))
                    .child(button("center-layout", match self.draft.center_layout {
                        CenterLayout::Offset => "Layout: Offset",
                        CenterLayout::Stack => "Layout: Stack",
                    }).on_click(cx.listener(|this, _, _, cx| {
                        this.draft.center_layout = match this.draft.center_layout {
                            CenterLayout::Offset => CenterLayout::Stack,
                            CenterLayout::Stack => CenterLayout::Offset,
                        };
                        this.message("Center layout updated. Save to activate.", false, cx);
                    })))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Offset (default) spreads background center windows out. Stack keeps them centered; H/L brings the next window forward. Sizing stays the same."))
                    .when(cfg!(any(target_os = "macos", target_os = "linux")), |el| el
                        .child(div().text_sm().child(if cfg!(target_os = "macos") { "Menu bar icon" } else { "Tray icon" }))
                        .child(button("tray-icon", match self.draft.tray_icon {
                            TrayIconStyle::Color => "Color",
                            TrayIconStyle::Monochrome => "Monochrome",
                        })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.tray_icon = match this.draft.tray_icon {
                                    TrayIconStyle::Color => TrayIconStyle::Monochrome,
                                    TrayIconStyle::Monochrome => TrayIconStyle::Color,
                                };
                                this.message("Tray icon updated. Save to activate.", false, cx);
                            })))
                        .child(div().text_xs().text_color(rgb(0x91a4bb)).child(if cfg!(target_os = "macos") {
                            "Monochrome follows the menu bar's light or dark appearance. Color shows the full app icon."
                        } else {
                            "Monochrome draws a white silhouette for dark panels. Color shows the full app icon."
                        })))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Window focus"))
                    .when(cfg!(windows), |el| el
                        .child(div().text_sm().child("Focus glow"))
                        .child(button("focus-glow", if self.draft.focus.glow_enabled { "On" } else { "Off" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft.focus.glow_enabled = !this.draft.focus.glow_enabled;
                                this.message("Focus glow updated. Save to activate.", false, cx);
                            })))
                        .child(div().flex().items_center().gap_2()
                            .child(div().size(px(18.)).rounded_sm().bg(rgb(self.draft.focus.glow_rgb().unwrap_or(0xf2f0e8))))
                            .child(button("glow-color", &format!("Color: {}", self.draft.focus.glow_color))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    #[cfg(windows)]
                                    {
                                        let initial = this.draft.focus.glow_rgb().unwrap_or(0xf2f0e8);
                                        cx.spawn(async move |this, cx| {
                                            // ChooseColorW pumps Windows messages. Run it after the
                                            // click listener releases GPUI's App/entity borrows so
                                            // activation observers and foreground tasks can re-enter.
                                            let color = platform::choose_glow_color(initial);
                                            if let Some(color) = color {
                                                let _ = this.update(cx, |this, cx| {
                                                    this.draft.focus.glow_color = format!("#{color:06X}");
                                                    this.message("Glow color updated. Save to activate.", false, cx);
                                                });
                                            }
                                        }).detach();
                                    }
                                    let _ = (this, cx);
                                }))))
                        .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Briefly highlight the target after moving a window or using H/J/K/L. Default: neutral off-white.")))
                    .child(div().text_sm().child("Include middle windows in H/J/K/L navigation"))
                    .child(button("include-middle", if self.draft.focus.include_middle_windows { "On" } else { "Off" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.focus.include_middle_windows = !this.draft.focus.include_middle_windows;
                            this.message("Window focus updated. Save to activate.", false, cx);
                        })))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Snapped window gaps"))
                    .child(div().text_sm().child("Inner gap per window"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("gap-inner-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.gaps.inner = this.draft.gaps.inner.saturating_sub(1);
                            this.message("Window gaps updated. Save to activate.", false, cx);
                        })))
                        .child(format!("{}", self.draft.gaps.inner))
                        .child(button("gap-inner-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.gaps.inner = (this.draft.gaps.inner + 1).min(100);
                            this.message("Window gaps updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_sm().child("Desktop edge gap"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("gap-outer-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.gaps.outer = this.draft.gaps.outer.saturating_sub(1);
                            this.message("Window gaps updated. Save to activate.", false, cx);
                        })))
                        .child(format!("{}", self.draft.gaps.outer))
                        .child(button("gap-outer-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.gaps.outer = (this.draft.gaps.outer + 1).min(100);
                            this.message("Window gaps updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Defaults: 5 on each inner edge (10 between tiles), 20 at desktop edges. Range: 0–100; 0 disables spacing. Applies on the next snap. Center windows are unchanged."))
                    .child(button("advanced-toggle", if self.advanced_expanded { "Advanced ▾" } else { "Advanced ▸" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.advanced_expanded = !this.advanced_expanded;
                            cx.notify();
                        })))
                    .when(self.advanced_expanded, |el| el
                    .child(div().text_sm().child("Filter middle windows by visibility"))
                    .child(button("visibility-filter", if self.draft.focus.visibility_filter_enabled { "On" } else { "Off" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.focus.visibility_filter_enabled = !this.draft.focus.visibility_filter_enabled;
                            this.message("Window focus updated. Save to activate.", false, cx);
                        })))
                    .child(div().text_sm().child("Minimum middle-window visibility"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("visibility-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.focus.minimum_visible_percent = this.draft.focus.minimum_visible_percent.saturating_sub(1);
                            this.message("Window focus updated. Save to activate.", false, cx);
                        })))
                        .child(format!("{}%", self.draft.focus.minimum_visible_percent))
                        .child(button("visibility-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.focus.minimum_visible_percent = (this.draft.focus.minimum_visible_percent + 1).min(99);
                            this.message("Window focus updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("When middle-window navigation is enabled, Filter Off includes covered windows. Filter On requires more than the percentage above (default: 10%) to be visible. Hold H/L for 400 ms to enter covered middle windows from an edge, or return from a middle window to the background. Minimized windows and other virtual desktops stay excluded."))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Width adjustment"))
                    .child(div().text_sm().child("Width change per press"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("width-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.width_resize.percent = this.draft.width_resize.percent.saturating_sub(1).max(1);
                            this.preview.select(this.preview.action, this.draft.width_resize.percent, Instant::now());
                            this.message("Width adjustment updated. Save to activate.", false, cx);
                        })))
                        .child(format!("{}%", self.draft.width_resize.percent))
                        .child(button("width-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.width_resize.percent = (this.draft.width_resize.percent + 1).min(99);
                            this.preview.select(this.preview.action, this.draft.width_resize.percent, Instant::now());
                            this.message("Width adjustment updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Shrink/Expand width keep height and the left edge, right edge, or center fixed. Repeats keep resizing; expansion stops at the work-area edge. Hover a position first to preview its width adjustment."))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Repeat shortcut to grow"))
                    .child(div().text_sm().child("Growth per press"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("growth-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.growth.percent = this.draft.growth.percent.saturating_sub(1).max(1);
                            this.message("Growth updated. Save to activate.", false, cx);
                        })))
                        .child(format!("{}%", self.draft.growth.percent))
                        .child(button("growth-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.growth.percent = (this.draft.growth.percent + 1).min(100);
                            this.message("Growth updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_sm().child("Growth steps before reset"))
                    .child(div().flex().items_center().justify_between()
                        .child(button("steps-less", "−").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.growth.steps = this.draft.growth.steps.saturating_sub(1);
                            this.message("Growth updated. Save to activate.", false, cx);
                        })))
                        .child(if self.draft.growth.steps == 0 { "Off".to_owned() } else { self.draft.growth.steps.to_string() })
                        .child(button("steps-more", "+").on_click(cx.listener(|this, _, _, cx| {
                            this.draft.growth.steps = (this.draft.growth.steps + 1).min(20);
                            this.message("Growth updated. Save to activate.", false, cx);
                        }))))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("First press places the window. Repeats multiply its size, keeping the corner or edge anchored. Center starts at 50% width and height, then grows equally in every direction. Growth stops at the work area; 0 steps turns it off.")))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child(if platform::is_gnome_wayland() {
                        "GNOME: saving registers these as GNOME keyboard shortcuts. The first time, log out and back in so GNOME loads the tt-pane extension."
                    } else if platform::is_wayland() {
                        "This Wayland compositor isn't supported yet. tt-pane runs on GNOME (Wayland) and in X11 sessions."
                    } else { "Closing settings keeps tt-pane running in the tray. Use the tray menu to quit." }))))
            .children({
                #[cfg(windows)]
                { Some(div().flex().flex_col().gap_2()
                    .child(div().text_sm().child(format!("Updates · Version {}", env!("CARGO_PKG_VERSION"))))
                    .child(div().text_xs().child(self.update.message.clone()))
                    .child(div().flex().gap_3()
                        .when(self.update.pending.is_none(), |el| el.child(button("check-update", "Check for updates").on_click(cx.listener(|this, _, _, cx| this.check_update(cx)))))
                        .when(self.update.pending.is_none() && self.update.release.is_some(), |el| el.child(button("install-update", "Update and restart").on_click(cx.listener(|this, _, _, cx| this.install_update(cx))))))) }
                #[cfg(not(windows))]
                { None::<Div> }
            })
            .child(div().text_sm().text_color(rgb(if self.error { 0xffa59b } else { 0x91a4bb })).child(self.status.clone()))
            .child(div().flex().items_center().justify_between().gap_3()
                .child(button("reset", "Reset defaults").on_click(cx.listener(|this, _, _, cx| {
                    this.recording = None;
                    let result = this.resume();
                    this.draft = Config::default();
                    this.preview.select(this.preview.action, this.draft.width_resize.percent, Instant::now());
                    this.message(result.err().map(|e| format!("{e:#}")).unwrap_or_else(|| "Defaults restored. Save to activate.".into()), false, cx);
                })))
                .child(div().flex().gap_3()
                    .when(platform::is_gnome_wayland(), |el| el.child(button("setup-gnome", "Set up GNOME").on_click(cx.listener(|this, _, _, cx| this.setup_gnome(cx)))))
                    .child(button("save", "Save changes").bg(rgb(0x217b68)).on_click(cx.listener(|this, _, _, cx| this.save(cx))))));
        window_frame(content, window)
    }
}

const WINDOW_TITLE: &str = "tt-pane — Keyboard shortcuts";
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
        let shortcuts = match Shortcuts::new() {
            Ok(mut shortcuts) => {
                if let Err(e) = shortcuts.replace(&config) {
                    status = format!("{e:#}");
                    error = true;
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
            recording: None,
            focus: cx.focus_handle(),
            status,
            error,
            preview: PositionPreview::new(Action::Left),
            advanced_expanded: false,
            window: None,
            _tray: tray,
            #[cfg(windows)]
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
            #[cfg(any(windows, target_os = "linux"))]
            let mut group_observed = std::time::Instant::now();
            // Centered groups need window inspection: native on Windows, the
            // Shell extension on GNOME. Other Linux sessions have none.
            #[cfg(any(windows, target_os = "linux"))]
            let observe_groups = cfg!(windows) || platform::is_gnome_wayland();
            loop {
                smol::Timer::after(Duration::from_millis(16)).await;
                if cx
                    .update(|cx| {
                        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                            #[cfg(windows)]
                            if event.id.0 == crate::tray::UPDATE {
                                if let Err(e) = show_settings(&settings, cx) {
                                    settings.update(cx, |this, cx| {
                                        this.message(format!("{e:#}"), true, cx)
                                    });
                                    continue;
                                }
                                settings.update(cx, |this, cx| {
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
                            #[cfg(windows)]
                            this.poll_update(cx);
                            #[cfg(any(windows, target_os = "linux"))]
                            if observe_groups
                                && group_observed.elapsed() >= Duration::from_millis(100)
                            {
                                group_observed = std::time::Instant::now();
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
