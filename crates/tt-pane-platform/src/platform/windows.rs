use crate::geometry::Rect;
mod glow;
use anyhow::{Result, ensure};
pub use glow::{choose_glow_color, show_focus_glow, wait_for_focus_glow};
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

/// A restored application window, identified by both process and native handle.
#[derive(Debug, Clone)]
pub struct WindowSnapshot {
    pub handle: u64,
    pub id: String,
    pub area: Rect,
    pub bounds: Rect,
}

pub fn foreground_handle() -> Option<u64> {
    // SAFETY: GetForegroundWindow returns an opaque handle and accepts no pointers.
    let window = unsafe { GetForegroundWindow() };
    (!window.is_invalid()).then_some(window.0 as usize as u64)
}

/// Read without restoring, activating, or repositioning a window. Unavailable
/// windows are deliberately excluded so group updates cannot resurrect them.
pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    let window = HWND(usize::try_from(handle)? as *mut _);
    let _dpi = DpiContext::enter();
    // SAFETY: Win32 validates the opaque handle; all output buffers are initialized
    // and correctly sized, and no pointers escape these synchronous calls.
    unsafe {
        if !IsWindow(Some(window)).as_bool()
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
            || IsZoomed(window).as_bool()
            || window == GetDesktopWindow()
            || window == GetShellWindow()
            || (GetWindowLongPtrW(window, GWL_EXSTYLE) as u32
                & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0))
                != 0
        {
            return Ok(None);
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(
            window,
            windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        )?;
        if cloaked != 0 {
            return Ok(None);
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        ensure!(
            GetMonitorInfoW(monitor, &mut info).as_bool(),
            "Could not read monitor work area"
        );
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        Ok(Some(WindowSnapshot {
            handle,
            id: format!("win32:{pid}:{}", window.0 as usize),
            area: from_rect(info.rcWork),
            bounds: from_rect(read_bounds(window)?.1),
        }))
    }
}

/// Revalidate immediately before a group move. Never restore an unavailable window
/// or overwrite a manual move since the coordinator's last observation.
pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    let live = window_snapshot(expected.handle)?
        .ok_or_else(|| anyhow::anyhow!("Window is no longer available"))?;
    ensure!(
        live.id == expected.id && live.area == expected.area && live.bounds == expected.bounds,
        "Window changed before the group move"
    );
    apply(Some(expected.handle), &mut |id, area, bounds| {
        ensure!(
            id == expected.id && area == expected.area && bounds == expected.bounds,
            "Window changed during the group move"
        );
        Ok(target)
    })
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    let live = window_snapshot(expected.handle)?
        .ok_or_else(|| anyhow::anyhow!("Window is no longer available"))?;
    ensure!(
        live.id == expected.id && live.area == expected.area && live.bounds == expected.bounds,
        "Window changed before focus"
    );
    let window = HWND(usize::try_from(expected.handle)? as *mut _);
    activate_window(window)
}

/// Cross-input-queue activation can still be pending after SetForegroundWindow
/// succeeds. Confirm it before callers sample foreground or arrange a group.
fn activate_window(window: HWND) -> Result<()> {
    // SAFETY: Win32 validates the opaque HWND; these calls retain no Rust data.
    unsafe {
        ensure!(
            SetForegroundWindow(window).as_bool(),
            "Windows refused to focus the window"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            if GetForegroundWindow() == window {
                return Ok(());
            }
            ensure!(
                IsWindow(Some(window)).as_bool(),
                "Window closed before activation completed"
            );
            ensure!(
                std::time::Instant::now() < deadline,
                "Window activation did not complete within 500 ms"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

pub fn apply(
    handle: Option<u64>,
    resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>,
) -> Result<Rect> {
    let window = match handle {
        Some(handle) => HWND(usize::try_from(handle)? as *mut _),
        // SAFETY: GetForegroundWindow takes no pointers and returns an opaque OS handle.
        None => unsafe { GetForegroundWindow() },
    };
    let _dpi = DpiContext::enter();
    apply_inner(window, resolve)
}

/// Discover live, visible neighbors rather than relying on saved placements.
pub fn resize_pair(handle: Option<u64>, up: bool) -> Result<()> {
    use windows::Win32::Foundation::LPARAM;
    use windows::core::BOOL;
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: EnumWindows invokes this synchronously with our exclusive live Vec.
        unsafe { &mut *(data.0 as *mut Vec<HWND>) }.push(window);
        BOOL(1)
    }
    let _dpi = DpiContext::enter();
    // SAFETY: All HWNDs are opaque identifiers validated by Win32; output buffers
    // have their declared size and remain live throughout each synchronous call.
    unsafe {
        let selected = match handle {
            Some(handle) => HWND(usize::try_from(handle)? as *mut _),
            None => GetForegroundWindow(),
        };
        ensure!(
            IsWindow(Some(selected)).as_bool(),
            "Invalid or missing window"
        );
        ensure!(
            !IsIconic(selected).as_bool() && !IsZoomed(selected).as_bool(),
            "Select a restored window to resize a shared boundary"
        );
        ensure!(
            selected != GetDesktopWindow() && selected != GetShellWindow(),
            "Desktop cannot be resized"
        );
        let current = from_rect(read_bounds(selected)?.1);
        let monitor = MonitorFromWindow(selected, MONITOR_DEFAULTTONEAREST);
        let mut windows = Vec::<HWND>::new();
        EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize))?;
        let mut candidates = Vec::new();
        for window in windows {
            if window == selected
                || window == GetDesktopWindow()
                || window == GetShellWindow()
                || !IsWindowVisible(window).as_bool()
                || IsIconic(window).as_bool()
                || IsZoomed(window).as_bool()
                || (GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) != 0
                || MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) != monitor
            {
                continue;
            }
            let mut cloaked = 0u32;
            if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                window,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                continue;
            }
            let Ok((_, bounds)) = read_bounds(window) else {
                continue;
            };
            let neighbor = from_rect(bounds);
            if let Some(targets) = crate::geometry::resize_vertical_pair(current, neighbor, up) {
                candidates.push((window, neighbor, targets));
            }
        }
        // In a stack, prefer the neighbor in the arrow's direction. At an outer
        // edge use the other neighbor, so either arrow adjusts the same pair.
        let preferred = candidates
            .iter()
            .any(|(_, rect, _)| (rect.y < current.y) == up);
        candidates.retain(|(_, rect, _)| !preferred || (rect.y < current.y) == up);
        ensure!(
            !candidates.is_empty(),
            "No vertically adjacent window with matching left and right edges on this monitor"
        );
        ensure!(
            candidates.len() == 1,
            "Multiple overlapping neighbors; shared boundary is ambiguous"
        );
        let (neighbor, before, (target, neighbor_target)) = candidates[0];
        apply_pair(selected, current, target, neighbor, before, neighbor_target)
    }
}

fn apply_pair(
    selected: HWND,
    before: Rect,
    target: Rect,
    neighbor: HWND,
    neighbor_before: Rect,
    neighbor_target: Rect,
) -> Result<()> {
    // Resize the shrinking window first. If an app enforces a minimum or rejects
    // either target, restore both original bounds and report the failure.
    let mut pair = [
        (selected, before, target),
        (neighbor, neighbor_before, neighbor_target),
    ];
    if target.height > before.height {
        pair.swap(0, 1);
    }
    let result = (|| -> Result<()> {
        for &(window, _, requested) in &pair {
            let actual = apply_inner(window, &mut |_, _, _| Ok(requested))?;
            ensure!(
                actual == requested,
                "Window minimum size or application constraints prevent this resize"
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for &(window, original, _) in pair.iter().rev() {
            match apply_inner(window, &mut |_, _, _| Ok(original)) {
                Ok(actual) if actual == original => {}
                Ok(_) => rollback_errors
                    .push("application did not restore its original bounds".to_owned()),
                Err(error) => rollback_errors.push(error.to_string()),
            }
        }
        ensure!(
            rollback_errors.is_empty(),
            "{error}; rollback incomplete: {}",
            rollback_errors.join("; ")
        );
        return Err(error);
    }
    Ok(())
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

fn apply_inner(
    window: HWND,
    resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>,
) -> Result<Rect> {
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
        let (mut outer, mut visible) = read_bounds(window)?;
        let current = from_rect(visible);
        let area = from_rect(info.rcWork);
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        let id = format!("win32:{pid}:{}", window.0 as usize);
        let target = resolve(&id, area, current)?;
        // Resolve fallible actions before restoring a minimized/maximized window.
        if IsZoomed(window).as_bool() || IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
            (outer, visible) = read_bounds(window)?;
        }
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
        GetWindowRect(window, &mut outer)?;
        let actual = Rect {
            x: outer.left + left,
            y: outer.top + top,
            width: outer.right - outer.left - left - right,
            height: outer.bottom - outer.top - top - bottom,
        };
        let aligned = current.align_width_result(target, actual);
        if aligned.x != actual.x {
            SetWindowPos(
                window,
                None,
                aligned.x - left,
                actual.y - top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
            GetWindowRect(window, &mut outer)?;
            return Ok(Rect {
                x: outer.left + left,
                y: outer.top + top,
                ..actual
            });
        }
        Ok(actual)
    }
}

fn read_bounds(window: HWND) -> Result<(RECT, RECT)> {
    let mut outer = RECT::default();
    // SAFETY: Win32 validates the opaque handle. Both output buffers remain live
    // and DWM receives the exact size of its RECT output for the synchronous call.
    unsafe {
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
        Ok((outer, visible))
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

/// Join an existing parent console for Ctrl+C, preserving inherited redirection.
/// Never allocates a console when started from Explorer or a shortcut.
pub fn attach_parent_console() {
    use windows::Win32::System::Console::*;
    // SAFETY: These APIs inspect/set process-owned OS handles and retain no Rust
    // pointers. Saved handles remain owned by the process across AttachConsole.
    unsafe {
        let streams = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|kind| {
            (
                kind,
                GetStdHandle(kind)
                    .ok()
                    .filter(|handle| !handle.is_invalid()),
            )
        });
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            for (kind, handle) in streams {
                if let Some(handle) = handle {
                    let _ = SetStdHandle(kind, handle);
                }
            }
            // Do not inherit a launcher's ignored-Ctrl+C setting.
            let _ = SetConsoleCtrlHandler(None, false);
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
        // Resource 101 is the same multi-resolution tt-pane ICO embedded by build.rs.
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

/// Enumerate in Z-order, excluding hidden/minimized/cloaked and utility windows.
fn focus_candidates(
    handle: Option<u64>,
    visibility: super::FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<(Rect, Vec<HWND>, Vec<Rect>)> {
    use windows::Win32::Foundation::LPARAM;
    use windows::core::BOOL;
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: EnumWindows synchronously supplies our exclusive live Vec.
        unsafe { &mut *(data.0 as *mut Vec<HWND>) }.push(window);
        BOOL(1)
    }
    unsafe extern "system" fn collect_monitor(
        _: windows::Win32::Graphics::Gdi::HMONITOR,
        _: windows::Win32::Graphics::Gdi::HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: EnumDisplayMonitors supplies a live RECT and synchronously
        // invokes this callback with our exclusively borrowed Vec.
        unsafe {
            (&mut *(data.0 as *mut Vec<Rect>)).push(from_rect(*rect));
        }
        BOOL(1)
    }
    let _dpi = DpiContext::enter();
    // SAFETY: HWNDs are opaque OS identifiers. Enumeration and DWM calls are
    // synchronous, with correctly sized live output buffers. No pointers escape.
    unsafe {
        let selected = match handle {
            Some(handle) => HWND(usize::try_from(handle)? as *mut _),
            None => GetForegroundWindow(),
        };
        ensure!(
            IsWindow(Some(selected)).as_bool(),
            "Invalid or missing window"
        );
        ensure!(
            selected != GetDesktopWindow() && selected != GetShellWindow(),
            "Select an application window to change focus"
        );
        ensure!(
            !IsIconic(selected).as_bool(),
            "Select a non-minimized window to change focus"
        );
        let current = from_rect(read_bounds(selected)?.1);
        let snapped_only = if visibility == super::FocusVisibility::Hold {
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            ensure!(
                GetMonitorInfoW(
                    MonitorFromWindow(selected, MONITOR_DEFAULTTONEAREST),
                    &mut info
                )
                .as_bool(),
                "Could not read the selected window's monitor"
            );
            !group.is_empty()
                || !crate::visibility::is_snapped(current, Some(from_rect(info.rcWork)))
        } else {
            false
        };
        let middle_only = visibility == super::FocusVisibility::Hold && !snapped_only;
        let visibility = if visibility == super::FocusVisibility::Hold {
            super::FocusVisibility::IncludeHidden
        } else {
            visibility
        };
        let mut windows = Vec::<HWND>::new();
        EnumWindows(Some(collect), LPARAM(&mut windows as *mut _ as isize))?;
        let mut screens = Vec::<Rect>::new();
        if visibility != super::FocusVisibility::IncludeHidden {
            ensure!(
                windows::Win32::Graphics::Gdi::EnumDisplayMonitors(
                    None,
                    None,
                    Some(collect_monitor),
                    LPARAM(&mut screens as *mut _ as isize)
                )
                .as_bool(),
                "Could not enumerate screens for window visibility"
            );
        }
        let mut above = Vec::new();
        let mut candidates = Vec::new();
        let mut bounds = Vec::new();
        for window in windows {
            if glow::is_glow(window)
                || window == GetDesktopWindow()
                || window == GetShellWindow()
                || !IsWindowVisible(window).as_bool()
                || IsIconic(window).as_bool()
            {
                continue;
            }
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(
                window,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                continue;
            }
            let Ok((_, rect)) = read_bounds(window) else {
                continue;
            };
            let rect = from_rect(rect);
            let is_middle = middle_members.contains(&(window.0 as usize as u64));
            let selectable = window != selected
                && ((!snapped_only && group.is_empty())
                    || (!group.contains(&(window.0 as usize as u64))
                        && MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST)
                            == MonitorFromWindow(selected, MONITOR_DEFAULTTONEAREST)))
                && (GetWindowLongPtrW(window, GWL_EXSTYLE) as u32
                    & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0))
                    == 0;
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let work_area = (selectable
                && (snapped_only
                    || middle_only
                    || visibility != super::FocusVisibility::IncludeHidden)
                && GetMonitorInfoW(
                    MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                    &mut info,
                )
                .as_bool())
            .then(|| from_rect(info.rcWork));
            if selectable
                && (!snapped_only || (!is_middle && crate::visibility::is_snapped(rect, work_area)))
                && (!middle_only || is_middle || !crate::visibility::is_snapped(rect, work_area))
                && (visibility != super::FocusVisibility::BackgroundOnly || !is_middle)
                && crate::visibility::focus_eligible(rect, work_area, &screens, &above, visibility)
            {
                candidates.push(window);
                bounds.push(rect);
            }
            // Even the selected window, utilities, and rejected candidates cover
            // windows beneath them. EnumWindows visits topmost windows first.
            if visibility != super::FocusVisibility::IncludeHidden {
                above.push(rect);
            }
        }
        Ok((current, candidates, bounds))
    }
}

pub fn focus_eligible_handles(handle: u64, visibility: super::FocusVisibility) -> Result<Vec<u64>> {
    let (_, candidates, _) = focus_candidates(Some(handle), visibility, &[], &[])?;
    Ok(candidates
        .into_iter()
        .map(|window| window.0 as usize as u64)
        .collect())
}

pub fn focus_window(
    handle: Option<u64>,
    action: crate::geometry::Action,
    visibility: super::FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<()> {
    let (current, candidates, bounds) =
        focus_candidates(handle, visibility, group, middle_members)?;
    let neighbor = if group.is_empty() {
        crate::geometry::focus_neighbor(current, &bounds, action)
    } else {
        crate::geometry::focus_group_exit(current, &bounds, action)
    };
    if let Some(index) = neighbor {
        activate_window(candidates[index])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::Foundation::{LPARAM, LRESULT, WPARAM},
        core::w,
    };

    unsafe extern "system" fn constrained_window(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_WINDOWPOSCHANGING {
            // SAFETY: Windows supplies a live WINDOWPOS for this synchronous message.
            let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
            if !position.flags.contains(SWP_NOSIZE) {
                position.cy = position.cy.clamp(100, 500);
            }
        }
        // SAFETY: Forward the original native message and its unchanged parameters.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    #[test]
    fn paired_resize_rolls_back_when_either_application_rejects_its_bounds() {
        struct OwnedWindow(HWND);
        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                // SAFETY: This test exclusively owns the window on its creating thread.
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let _dpi = DpiContext::enter();
        // SAFETY: The class callback is valid for the process lifetime. These hidden
        // disposable windows are owned by this thread; all buffers are initialized.
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(constrained_window),
                lpszClassName: w!("tt-pane paired constraint test"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let create = |y| {
                OwnedWindow(
                    CreateWindowExW(
                        Default::default(),
                        class.lpszClassName,
                        w!("tt-pane test"),
                        WS_POPUP,
                        100,
                        y,
                        317,
                        400,
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                )
            };
            let top = create(100);
            let bottom = create(500);
            let a = from_rect(read_bounds(top.0).unwrap().1);
            let b = from_rect(read_bounds(bottom.0).unwrap().1);
            // The top accepts 200, then the bottom refuses 600 (max 500).
            let ta = Rect { height: 200, ..a };
            let tb = Rect {
                y: a.y + 200,
                height: 600,
                ..b
            };
            assert!(apply_pair(top.0, a, ta, bottom.0, b, tb).is_err());
            assert_eq!(from_rect(read_bounds(top.0).unwrap().1), a);
            assert_eq!(from_rect(read_bounds(bottom.0).unwrap().1), b);
            // The first (shrinking) window refuses a height below its minimum.
            let ta = Rect { height: 50, ..a };
            let tb = Rect {
                y: a.y + 50,
                height: 750,
                ..b
            };
            assert!(apply_pair(top.0, a, ta, bottom.0, b, tb).is_err());
            assert_eq!(from_rect(read_bounds(top.0).unwrap().1), a);
            assert_eq!(from_rect(read_bounds(bottom.0).unwrap().1), b);
            drop(bottom);
            drop(top);
            UnregisterClassW(class.lpszClassName, None).unwrap();
        }
    }
}
