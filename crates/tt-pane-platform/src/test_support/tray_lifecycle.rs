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
        // SAFETY: EnumWindows synchronously passes the live Search below, with
        // exclusive callback access; the pointer is never retained.
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut pid = 0;
        // SAFETY: pid is a live writable u32 and window is an opaque OS handle.
        unsafe {
            GetWindowThreadProcessId(window, Some(&mut pid));
        }
        // SAFETY: IsWindowVisible only inspects an opaque handle.
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
    // SAFETY: search stays alive and exclusively borrowed until enumeration ends.
    unsafe {
        EnumWindows(Some(visit), LPARAM(&mut search as *mut _ as isize)).unwrap();
    }
    search.windows
}

fn launch(binary: &str, settings: bool, config: &std::path::Path) -> Process {
    let mut cmd = Command::new(binary);
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
pub fn tray_starts_without_window_and_survives_settings_close(
    binary: &str,
    config: &std::path::Path,
) {
    std::fs::write(config, "version = 1\nbindings = []\n").unwrap();
    let mut tray = launch(binary, false, config);
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

    let mut settings = launch(binary, true, config);
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
    // SAFETY: WM_GETICON and WM_CLOSE carry only integers, not Rust pointers.
    // These handles identify the child process windows; expired handles fail in Win32.
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

/// Called only from the isolated hidden-console integration-test subprocess.
pub fn terminal_ctrl_c_terminates_tray_app(binary: &str) {
    use windows::Win32::System::Console::*;
    fn console_pids() -> Vec<u32> {
        let mut ids = [0u32; 16];
        // SAFETY: The API receives a live writable slice of process IDs.
        let count = unsafe { GetConsoleProcessList(&mut ids) } as usize;
        assert!(
            count > 0 && count <= ids.len(),
            "Expected a small private console"
        );
        ids[..count].to_vec()
    }
    assert_eq!(
        console_pids(),
        [std::process::id()],
        "Refusing to signal a shared console"
    );
    // A real parent console must not steal redirected CLI JSON output.
    let output = Command::new(binary).arg("actions").output().unwrap();
    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert!(
        response["actions"]
            .as_array()
            .is_some_and(|actions| !actions.is_empty())
    );
    // No arguments reproduces cargo run; null output reproduces inherited handles
    // that previously prevented attachment. No user configuration is modified.
    let mut child = Process(
        Command::new(binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !console_pids().contains(&child.0.id()) {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "App exited before attaching"
        );
        assert!(
            Instant::now() < deadline,
            "App never attached to the parent console"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Allow GPUI/tray startup, verifying it is still alive before interruption.
    std::thread::sleep(Duration::from_secs(3));
    assert!(child.0.try_wait().unwrap().is_none());
    let ids = console_pids();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&std::process::id()) && ids.contains(&child.0.id()));
    // SAFETY: The console is verified to contain only this helper and its owned
    // child. Ignore Ctrl+C in the helper only, after the child has been spawned.
    unsafe {
        SetConsoleCtrlHandler(None, true).unwrap();
        GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert_eq!(status.code().map(|code| code as u32), Some(0xc000013a));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Ctrl+C did not terminate the tray app"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
