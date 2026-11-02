use std::process::Command;
pub fn binary_positions_disposable_window_on_every_monitor(
    binary: &str,
    config: &str,
    percent: u32,
    steps: u32,
    width_percent: u32,
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
                w!("sTile native test"),
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
        let apply_command = |action: Action| {
            let mut child = Command::new(binary)
                .args([
                    "--config",
                    config,
                    "apply",
                    action.name(),
                    "--window",
                    &(window.0.0 as usize).to_string(),
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
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["ok"], true);
            let mut actual = RECT::default();
            GetWindowRect(window.0, &mut actual).unwrap();
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
                    let actual = apply_command(action);
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
                        expected = expected.adjust_width_from(area, anchor, change, base);
                        let actual = apply_command(adjustment);
                        assert_eq!(
                            actual, expected,
                            "{adjustment:?} from {action:?} on {area:?}"
                        );
                    }
                    assert_eq!(
                        expected.width, area.width,
                        "Expansion should stop at the work-area edge"
                    );
                }
            }
        }
        eprintln!(
            "Verified {} actions, {} growth steps plus reset, on {} monitors and shrink/expand sequences through separate CLI invocations",
            Action::POSITIONS.len(),
            steps,
            monitors.len()
        );
        if !previous.is_invalid() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}
