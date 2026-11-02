use crate::geometry::{Action, Rect};
use anyhow::{Result, ensure};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    },
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
        WindowsAndMessaging::*,
    },
};

pub fn apply(action: Action) -> Result<()> {
    apply_to(action, unsafe { GetForegroundWindow() })
}

pub fn apply_to(action: Action, window: HWND) -> Result<()> {
    // A thread-local context avoids DPI virtualization, including mixed-scale monitors.
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result = apply_inner(action, window);
        if !previous.is_invalid() {
            SetThreadDpiAwarenessContext(previous);
        }
        result
    }
}

unsafe fn apply_inner(action: Action, window: HWND) -> Result<()> {
    unsafe {
        ensure!(
            !window.is_invalid() && IsWindow(Some(window)).as_bool(),
            "Invalid or missing window"
        );
        ensure!(
            window != GetDesktopWindow() && window != GetShellWindow(),
            "Desktop cannot be positioned"
        );
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        ensure!(
            GetMonitorInfoW(monitor, &mut info).as_bool(),
            "Could not read monitor work area"
        );
        if IsZoomed(window).as_bool() || IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let mut outer = RECT::default();
        GetWindowRect(window, &mut outer)?;
        let mut visible = outer;
        let result = DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if result.is_err() || visible.right <= visible.left || visible.bottom <= visible.top {
            visible = outer;
        }
        let area = from_rect(info.rcWork);
        let target = action.target(area, from_rect(visible));
        // Account for invisible resize borders so visible edges align with the work area.
        let left = visible.left - outer.left;
        let top = visible.top - outer.top;
        let right = outer.right - visible.right;
        let bottom = outer.bottom - visible.bottom;
        SetWindowPos(
            window,
            None,
            target.x - left,
            target.y - top,
            target.width + left + right,
            target.height + top + bottom,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        Ok(())
    }
}

fn from_rect(rect: RECT) -> Rect {
    Rect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    }
}
