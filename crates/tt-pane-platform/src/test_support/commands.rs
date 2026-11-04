use std::process::Command;
pub fn binary_positions_disposable_window_on_every_monitor(
    binary: &str,
    config: &str,
    percent: u32,
    steps: u32,
    width_percent: u32,
    gaps: crate::geometry::SnapGaps,
) {
    use crate::geometry::{Action, Rect};
    use std::time::{Duration, Instant};
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
            UI::{HiDpi::*, WindowsAndMessaging::*},
        },
        core::{BOOL, w},
    };

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: EnumDisplayMonitors synchronously passes our live Vec pointer,
        // with exclusive callback access, and never retains it.
        let monitors = unsafe { &mut *(data.0 as *mut Vec<RECT>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: info is initialized and cbSize matches its allocation.
        if unsafe { GetMonitorInfoW(monitor, &mut info).as_bool() } {
            monitors.push(info.rcWork);
        }
        BOOL(1)
    }
    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: This guard uniquely owns the window on its creating thread.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    // SAFETY: Enumeration is synchronous with a live exclusive Vec pointer. The
    // hidden STATIC window is owned by this thread and destroyed by its guard.
    // All output buffers are initialized stack structs and message pointers remain
    // valid for synchronous dispatch; no user window or borrowed memory is owned.
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut monitors = Vec::<RECT>::new();
        assert!(
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut _ as isize)
            )
            .as_bool()
        );
        assert!(!monitors.is_empty());
        // Never shown or activated; no input injection and no user window is touched.
        let window = TestWindow(
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("tt-pane native test"),
                WS_POPUP,
                0,
                0,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
        );
        let destination = TestWindow(
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("tt-pane gap destination test"),
                WS_POPUP,
                0,
                0,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
        );
        let apply_command = |action: Action, target: HWND, succeed: bool| {
            let state_path = format!("{config}.cycle.json");
            let state_before = std::fs::read(&state_path).ok();
            let mut child = Command::new(binary)
                .args([
                    "--config",
                    config,
                    "apply",
                    action.name(),
                    "--window",
                    &(target.0 as usize).to_string(),
                    "--json",
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                // SetWindowPos can send messages to the owner thread. Pump that
                // thread while the external CLI process executes the native call.
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("CLI positioning timed out");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let output = child.wait_with_output().unwrap();
            assert_eq!(
                output.status.success(),
                succeed,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["ok"], succeed);
            if !succeed {
                assert_eq!(
                    std::fs::read(&state_path).ok(),
                    state_before,
                    "Failed actions must preserve the saved gap"
                );
            }
            let mut actual = RECT::default();
            GetWindowRect(target, &mut actual).unwrap();
            Rect {
                x: actual.left,
                y: actual.top,
                width: actual.right - actual.left,
                height: actual.bottom - actual.top,
            }
        };
        for monitor in &monitors {
            let area = Rect {
                x: monitor.left,
                y: monitor.top,
                width: monitor.right - monitor.left,
                height: monitor.bottom - monitor.top,
            };
            for action in Action::POSITIONS {
                let current = Rect {
                    x: area.x + 40,
                    y: area.y + 40,
                    width: 320,
                    height: 240,
                };
                // Reproduce left-half -> Center: Center must reduce the full height.
                let current = if action == Action::Center {
                    Action::Left.target(area, current)
                } else {
                    current
                };
                SetWindowPos(
                    window.0,
                    None,
                    current.x,
                    current.y,
                    current.width,
                    current.height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .unwrap();
                let base = action.target(area, current);
                for step in 0..=steps + 1 {
                    let expected = if step == steps + 1 || step == 0 {
                        base
                    } else {
                        action.grown_target(
                            area,
                            base,
                            (1.0 + f64::from(percent) / 100.0).powi(step as i32),
                        )
                    };
                    let expected = if action == Action::Center {
                        expected
                    } else {
                        gaps.inset(area, expected)
                    };
                    let actual = apply_command(action, window.0, true);
                    assert_eq!(
                        actual, expected,
                        "{action:?}, growth step {step} on {area:?}"
                    );
                }
                if matches!(
                    action,
                    Action::BottomLeft | Action::BottomRight | Action::Center
                ) {
                    let anchor = action.horizontal_anchor().unwrap();
                    let resize_area = if action == Action::Center {
                        area
                    } else {
                        gaps.work_area(area)
                    };
                    let base = if action == Action::Center {
                        base
                    } else {
                        gaps.inset(area, base)
                    };
                    let mut expected = base;
                    for adjustment in [Action::ShrinkWidth; 4]
                        .into_iter()
                        .chain([Action::ExpandWidth; 12])
                    {
                        let change = width_percent as i32
                            * if adjustment == Action::ShrinkWidth {
                                -1
                            } else {
                                1
                            };
                        expected = expected.adjust_width_from(resize_area, anchor, change, base);
                        let actual = apply_command(adjustment, window.0, true);
                        assert_eq!(
                            actual, expected,
                            "{adjustment:?} from {action:?} on {area:?}"
                        );
                    }
                    assert_eq!(
                        expected.width, resize_area.width,
                        "Expansion should stop at the work-area edge"
                    );
                    let base = apply_command(action, window.0, true);
                    let reduced = apply_command(Action::ShrinkWidth, window.0, true);
                    let gap = base.freed_width_gap(reduced).unwrap();
                    let gap = if action == Action::Center {
                        gap
                    } else if gap.x >= reduced.x + reduced.width {
                        crate::geometry::inset_rect(gap, gaps.inner * 2, 0, 0, 0)
                    } else {
                        crate::geometry::inset_rect(gap, 0, 0, gaps.inner * 2, 0)
                    };
                    assert_eq!(apply_command(Action::FillGap, window.0, false), reduced);
                    SetWindowPos(
                        destination.0,
                        None,
                        area.x + 40,
                        area.y + 40,
                        320,
                        240,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                    .unwrap();
                    assert_eq!(
                        apply_command(Action::FillGap, destination.0, true),
                        gap,
                        "Fill gap after {width_percent}% reduction from {action:?} on {area:?}"
                    );
                    let mut source = RECT::default();
                    GetWindowRect(window.0, &mut source).unwrap();
                    assert_eq!(
                        Rect {
                            x: source.left,
                            y: source.top,
                            width: source.right - source.left,
                            height: source.bottom - source.top
                        },
                        reduced
                    );
                    assert_eq!(apply_command(Action::FillGap, destination.0, false), gap);
                }
            }
        }
        // A narrow, offset pair avoids matching any normal tiled user windows.
        // Only the disposable neighbor is briefly shown, without activation.
        let monitor = monitors[0];
        let top = Rect {
            x: monitor.left + 37,
            y: monitor.top + 43,
            width: 317,
            height: 240,
        };
        let bottom = Rect {
            y: top.y + top.height,
            ..top
        };
        for (handle, rect) in [(window.0, bottom), (destination.0, top)] {
            SetWindowPos(
                handle,
                None,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .unwrap();
        }
        assert_eq!(apply_command(Action::ResizeUp, window.0, false), bottom);
        SetWindowLongPtrW(destination.0, GWL_EXSTYLE, WS_EX_NOACTIVATE.0 as isize);
        let foreground = GetForegroundWindow();
        let _ = ShowWindow(destination.0, SW_SHOWNOACTIVATE);
        let mut pair = (bottom, top);
        for (action, expected_height) in [
            (Action::ResizeUp, 300),
            (Action::ResizeDown, 240),
            (Action::ResizeDown, 180),
            (Action::ResizeUp, 240),
        ] {
            pair =
                crate::geometry::resize_vertical_pair(pair.0, pair.1, action == Action::ResizeUp)
                    .unwrap();
            assert_eq!(pair.0.height, expected_height);
            assert_eq!(apply_command(action, window.0, true), pair.0);
            let mut actual = RECT::default();
            GetWindowRect(destination.0, &mut actual).unwrap();
            assert_eq!(
                (
                    actual.left,
                    actual.top,
                    actual.right - actual.left,
                    actual.bottom - actual.top
                ),
                (pair.1.x, pair.1.y, pair.1.width, pair.1.height)
            );
            assert!(!std::path::Path::new(&format!("{config}.cycle.json")).exists());
        }
        assert_eq!(pair, (bottom, top));
        assert_eq!(GetForegroundWindow(), foreground);
        let _ = ShowWindow(destination.0, SW_HIDE);
        assert_eq!(apply_command(Action::ResizeDown, window.0, false), pair.0);
        eprintln!(
            "Verified {} actions, {} growth steps plus reset, on {} monitors and shrink/expand/two-window gap sequences through separate CLI invocations",
            Action::POSITIONS.len(),
            steps,
            monitors.len()
        );
        if !previous.is_invalid() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}
