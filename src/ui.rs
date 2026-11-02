use crate::{
    config::{Binding, Config},
    geometry::Action,
    platform,
    shortcuts::{Shortcuts, wayland_bindings},
};
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
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
    window: Option<WindowHandle<Settings>>,
    _tray: tray_icon::TrayIcon,
}

impl Settings {
    fn message(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.status = text.into();
        self.error = error;
        let _ = self._tray.set_tooltip(Some(if error {
            "sTile — action failed; open Settings for details"
        } else {
            "sTile — window shortcuts"
        }));
        cx.notify();
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
        self.active = self.draft.clone();
        self.message(
            if platform::is_wayland() {
                "Saved. Export shortcuts and reload your compositor to activate them."
            } else {
                "Saved. Your shortcuts are active across all applications."
            },
            false,
            cx,
        );
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<PathBuf> {
            let compositor = if std::env::var_os("SWAYSOCK").is_some() {
                "sway"
            } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
                "hyprland"
            } else {
                anyhow::bail!("Start sTile inside Sway or Hyprland, or use the export command")
            };
            let text = wayland_bindings(
                &self.active,
                compositor,
                &std::env::current_exe()?,
                Some(&std::path::absolute(&self.path)?),
            )?;
            let path = self.path.with_file_name(format!("stile-{compositor}.conf"));
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, text)?;
            Ok(path)
        })();
        match result {
            Ok(path) => self.message(
                format!(
                    "Exported {}. Include it in your compositor config and reload.",
                    path.display()
                ),
                false,
                cx,
            ),
            Err(error) => self.message(format!("{error:#}"), true, cx),
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
        div().track_focus(&self.focus).on_key_down(cx.listener(Self::key_down))
            .size_full().bg(rgb(0x0e1724)).text_color(rgb(0xe8eef7)).font_family("Segoe UI")
            .flex().flex_col().p_6().gap_4()
            .child(div().flex().items_center().justify_between()
                .child(div().flex().flex_col().gap_1().child(div().text_3xl().font_weight(FontWeight::BOLD).child("sTile"))
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
                    .when(selected.focuses(), |el| el.child(div().text_xs().text_color(rgb(0x6ee7c0)).child("Focus the nearest window in this direction without moving or resizing it. H: left, J: down, K: up, L: right. Windows only; stops at the edge.")))
                    .when(selected == Action::FillGap, |el| el.child(div().text_xs().text_color(rgb(0x6ee7c0)).child("Example gap after shrinking the last previewed position. To fill a real gap, shrink a window, select another on the same monitor, then press Ctrl+Alt+Shift+8 (*).")))
                    .child(div().text_sm().text_color(rgb(0x91a4bb)).child("Positions use the active window’s monitor and its available work area. A window spanning displays uses the monitor with the largest overlap."))
                    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Window focus"))
                    .child(div().text_sm().child("Include obscured windows"))
                    .child(button("include-obscured", if self.draft.focus.include_obscured { "On" } else { "Off" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft.focus.include_obscured = !this.draft.focus.include_obscured;
                            this.message("Window focus updated. Save to activate.", false, cx);
                        })))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("Off: at least 50% of a window must be on-screen and uncovered. On: also include covered windows. Minimized windows and other virtual desktops stay excluded."))
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
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child("First press places the window. Repeats multiply its size, keeping the corner or edge anchored. Center starts at 50% width and height, then grows equally in every direction. Growth stops at the work area; 0 steps turns it off."))
                    .child(div().text_xs().text_color(rgb(0x91a4bb)).child(if platform::is_wayland() {
                        "Wayland: save, export shortcuts, then include the file in your compositor config. Sway and Hyprland are supported."
                    } else { "Closing settings keeps sTile running in the tray. Use the tray menu to quit." }))
                    .child(button("close", "Close settings").on_click(|_, window, _| window.remove_window()))))
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
                    .when(platform::is_wayland(), |el| el.child(button("export", "Export shortcuts").on_click(cx.listener(|this, _, _, cx| this.export(cx)))))
                    .child(button("save", "Save changes").bg(rgb(0x217b68)).on_click(cx.listener(|this, _, _, cx| this.save(cx))))))
    }
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
                title: Some("sTile — Keyboard shortcuts".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(600.))),
            app_id: Some("dev.stile.Stile".into()),
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
    // A persistent entity owns the shortcuts and tray independently of any window.
    let failure = std::rc::Rc::new(std::cell::RefCell::new(None));
    let result = failure.clone();
    Application::new().run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
            let mtm = objc2::MainThreadMarker::new().expect("GPUI main thread");
            NSApplication::sharedApplication(mtm)
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
        let tray = match crate::tray::create() {
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
        if error {
            let _ = tray.set_tooltip(Some(format!("sTile — {status}")));
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
            window: None,
            _tray: tray,
        });
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
            loop {
                smol::Timer::after(Duration::from_millis(16)).await;
                if cx
                    .update(|cx| {
                        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
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
                            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                                if event.state != HotKeyState::Pressed || this.recording.is_some() {
                                    continue;
                                }
                                if let Some(action) =
                                    this.shortcuts.as_ref().and_then(|s| s.action(event.id))
                                {
                                    match crate::positioning::apply(
                                        &this.active,
                                        &this.path,
                                        action,
                                        None,
                                    ) {
                                        Ok(()) => this.message(
                                            format!("Positioned: {}", action.label()),
                                            false,
                                            cx,
                                        ),
                                        Err(e) => this.message(format!("{e:#}"), true, cx),
                                    }
                                }
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
