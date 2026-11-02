#![cfg(all(target_os = "windows", feature = "gui"))]

use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::*,
    },
    core::BOOL,
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn windows_for(pid: u32, visible: bool) -> Vec<HWND> {
    struct Search {
        pid: u32,
        visible: bool,
        windows: Vec<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, Some(&mut pid));
        }
        if pid == search.pid && (!search.visible || unsafe { IsWindowVisible(window).as_bool() }) {
            search.windows.push(window);
        }
        BOOL(1)
    }
    let mut search = Search {
        pid,
        visible,
        windows: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(visit), LPARAM(&mut search as *mut _ as isize)).unwrap();
    }
    search.windows
}

fn launch(settings: bool, config: &std::path::Path) -> Process {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_stile"));
    cmd.arg("--config").arg(config);
    if settings {
        cmd.arg("settings");
    }
    Process(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

// Requires a logged-in desktop and GPU. No Computer Use, mouse, or keyboard input.
#[test]
#[ignore = "requires an interactive Windows desktop"]
fn tray_starts_without_window_and_survives_settings_close() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.toml");
    std::fs::write(&config, "version = 1\nbindings = []\n").unwrap();
    let mut tray = launch(false, &config);
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        tray.0.try_wait().unwrap().is_none(),
        "Tray process exited during startup"
    );
    assert!(
        windows_for(tray.0.id(), true).is_empty(),
        "Startup created a visible window"
    );
    assert!(
        !windows_for(tray.0.id(), false).is_empty(),
        "Expected native event/tray infrastructure"
    );
    drop(tray);

    let mut settings = launch(true, &config);
    let deadline = Instant::now() + Duration::from_secs(15);
    let window = loop {
        if let Some(window) = windows_for(settings.0.id(), true).first() {
            break *window;
        }
        assert!(
            settings.0.try_wait().unwrap().is_none(),
            "Settings process exited"
        );
        assert!(Instant::now() < deadline, "Settings did not open");
        std::thread::sleep(Duration::from_millis(50));
    };
    unsafe {
        for size in [ICON_SMALL, ICON_BIG] {
            let icon = SendMessageW(
                window,
                WM_GETICON,
                Some(WPARAM(size as usize)),
                Some(LPARAM(0)),
            );
            assert_ne!(
                icon.0, 0,
                "Settings must have a native title-bar/task-switcher icon"
            );
        }
        PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !windows_for(settings.0.id(), true).is_empty() {
        assert!(Instant::now() < deadline, "Settings failed to close");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        settings.0.try_wait().unwrap().is_none(),
        "Closing settings quit the tray process"
    );
}
