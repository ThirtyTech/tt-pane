use std::process::Command;

#[test]
fn discovery_matches_plugin_manifest_without_loading_config() {
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
        .args([
            "--config",
            "nonexistent-do-not-create/config.toml",
            "actions",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    let manifest: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
    let actions = manifest["actions"].as_array().unwrap();
    assert_eq!(response["actions"].as_array().unwrap().len(), actions.len());
    for action in actions {
        let id = action["id"].as_str().unwrap();
        assert!(
            response["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == id)
        );
        assert_eq!(action["command"][2].as_str(), Some(id));
    }
}

#[test]
fn invalid_target_returns_json_and_failure_exit_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
        .args(["apply", "left", "--window", "0", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"].is_string());
}

#[cfg(target_os = "windows")]
#[test]
fn binary_positions_disposable_window_on_every_monitor() {
    use std::time::{Duration, Instant};
    use stile::geometry::{Action, Rect};
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
        let monitors = unsafe { &mut *(data.0 as *mut Vec<RECT>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info).as_bool() } {
            monitors.push(info.rcWork);
        }
        BOOL(1)
    }
    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
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
        for monitor in &monitors {
            let area = Rect {
                x: monitor.left,
                y: monitor.top,
                width: monitor.right - monitor.left,
                height: monitor.bottom - monitor.top,
            };
            for action in Action::ALL {
                let current = Rect {
                    x: area.x + 40,
                    y: area.y + 40,
                    width: 320,
                    height: 240,
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
                let expected = action.target(area, current);
                let mut child = Command::new(env!("CARGO_BIN_EXE_stile"))
                    .args([
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
                assert_eq!(
                    Rect {
                        x: actual.left,
                        y: actual.top,
                        width: actual.right - actual.left,
                        height: actual.bottom - actual.top
                    },
                    expected,
                    "{action:?} on {area:?}"
                );
            }
        }
        eprintln!(
            "Verified {} actions on {} monitors through the CLI",
            Action::ALL.len(),
            monitors.len()
        );
        if !previous.is_invalid() {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}
