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
    // SAFETY: GetForegroundWindow takes no pointers and returns an opaque OS handle.
    apply_to(action, unsafe { GetForegroundWindow() })
}

pub fn apply_to(action: Action, window: HWND) -> Result<()> {
    let _dpi = DpiContext::enter();
    apply_inner(action, window)
}

// A thread-bound guard restores DPI awareness on errors and unwinding too.
struct DpiContext {
    previous: windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl DpiContext {
    fn enter() -> Self {
        // SAFETY: This constant is a valid context; the call only changes this thread.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self {
            previous,
            _thread_bound: std::marker::PhantomData,
        }
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        if !self.previous.is_invalid() {
            // SAFETY: The context came from this thread's successful setter call.
            // The guard cannot be sent to another thread.
            unsafe {
                SetThreadDpiAwarenessContext(self.previous);
            }
        }
    }
}

fn apply_inner(action: Action, window: HWND) -> Result<()> {
    // SAFETY: HWNDs are opaque OS identifiers, never dereferenced by Rust. Invalid
    // or expired handles are rejected by Win32. All output buffers below are live,
    // initialized structs with the exact sizes required by their APIs; no pointer
    // escapes these synchronous calls. SetWindowPos carries no application pointer.
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

/// Attach CLI output to the parent console only when no output handle exists.
pub fn attach_parent_console() {
    use windows::Win32::System::Console::*;
    // SAFETY: Both functions use only OS handles/constants and retain no Rust pointers.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).map_or(true, |handle| handle.is_invalid()) {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(feature = "window-icon")]
/// Set the executable's shared icon resource (101) on a borrowed native window.
pub fn set_window_icon(window: &impl raw_window_handle::HasWindowHandle) -> anyhow::Result<()> {
    use raw_window_handle::RawWindowHandle;
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{ICON_BIG, ICON_SMALL, LoadIconW, SendMessageW, WM_SETICON},
        },
        core::PCWSTR,
    };
    let borrowed = window
        .window_handle()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let RawWindowHandle::Win32(handle) = borrowed.as_raw() else {
        anyhow::bail!("Expected a native Windows handle");
    };
    // SAFETY: The borrowed handle keeps the window valid for these synchronous calls.
    // Resource 101 uses Win32 MAKEINTRESOURCE encoding (not a dereferenced pointer).
    // LoadIconW returns a module-owned shared icon that outlives WM_SETICON usage.
    unsafe {
        // Resource 101 is the same multi-resolution sTile ICO embedded by build.rs.
        // LoadIconW returns a shared resource owned by the module, not this window.
        let module = GetModuleHandleW(None)?;
        let icon = LoadIconW(Some(HINSTANCE(module.0)), PCWSTR(101usize as *const u16))?;
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        for size in [ICON_SMALL, ICON_BIG] {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(size as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
        }
    }
    Ok(())
}
