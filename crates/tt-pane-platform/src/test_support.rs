//! Native integration-test scenarios. Enabled only by the test-support feature.
mod commands;
mod tray_lifecycle;
pub use commands::binary_positions_disposable_window_on_every_monitor;
pub use tray_lifecycle::tray_starts_without_window_and_survives_settings_close;

pub use tray_lifecycle::terminal_ctrl_c_terminates_tray_app;

/// Exercise group placement with three disposable, non-activated native windows.
pub fn with_group_windows(run: impl FnOnce([u64; 3])) {
    use windows::{
        Win32::{Foundation::HWND, UI::WindowsAndMessaging::*},
        core::w,
    };
    struct Owned(HWND);
    struct RestoreFocus(HWND);
    impl Drop for RestoreFocus {
        fn drop(&mut self) {
            // SAFETY: The borrowed opaque handle is validated by Win32. This only
            // restores the original foreground window after testing our own windows.
            unsafe {
                let _ = SetForegroundWindow(self.0);
            }
        }
    }
    // SAFETY: No pointers or arguments; the OS owns the returned opaque handle.
    let _restore = RestoreFocus(unsafe { GetForegroundWindow() });
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: This guard exclusively owns the window on its creating thread.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    // SAFETY: STATIC is an OS-provided window class. These temporary windows are
    // owned by this thread, shown without activation, and destroyed by their guards.
    let windows: [Owned; 3] = std::array::from_fn(|_| unsafe {
        let window = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("tt-pane centered group test"),
            WS_POPUP,
            100,
            100,
            400,
            300,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        Owned(window)
    });
    run(windows.each_ref().map(|w| w.0.0 as usize as u64));
}
