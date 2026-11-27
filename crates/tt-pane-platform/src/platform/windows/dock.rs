//! The dock panel: a topmost AppBar on the primary monitor that shows live DWM
//! thumbnails of docked windows. Docked windows are parked off screen at 1080p,
//! where DWM keeps composing them, and carry a property so every other
//! tt-pane feature (tiling, centered groups, focus) leaves them alone.
use super::{DpiContext, from_rect};
use crate::{
    dock::{self, DockEdge, DockFill},
    geometry::Rect,
};
use anyhow::{Result, ensure};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::Mutex,
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

/// What the dock panel should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockRequest {
    pub edge: DockEdge,
    /// The end of the edge the panel grows from.
    pub fill: DockFill,
    pub percent: u32,
    /// Docked windows in slot order, from the fill end.
    pub members: Vec<u64>,
    /// The member currently shown in the arena.
    pub active: Option<u64>,
    /// A window that isn't docked is being dragged: show an empty slot to drop it on.
    pub dragging: bool,
    /// The dragged window is over the panel.
    pub highlight: bool,
}

/// Where the panel and its slots ended up, in physical screen pixels. There is
/// no layout while the panel is hidden, with nothing docked or dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockLayout {
    pub panel: Rect,
    pub slots: Vec<(u64, Rect)>,
}

struct Shared {
    request: Option<DockRequest>,
    layout: Option<DockLayout>,
    clicks: Vec<u64>,
    window: usize,
    thread: Option<JoinHandle<()>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    request: None,
    layout: None,
    clicks: Vec::new(),
    window: 0,
    thread: None,
});

const CLASS: PCWSTR = w!("tt-pane.dock");
const PANEL_PROP: PCWSTR = w!("tt-pane.dock-panel");
const DOCKED_PROP: PCWSTR = w!("tt-pane.docked");
const WM_DOCK_UPDATE: u32 = WM_APP + 0x40;
const WM_DOCK_APPBAR: u32 = WM_APP + 0x41;
const WM_DOCK_REPOSITION: u32 = WM_APP + 0x42;

const BACKGROUND: COLORREF = COLORREF(0x001C_1818);
const SLOT: COLORREF = COLORREF(0x0042_3C3C);
const ACCENT: COLORREF = COLORREF(0x00E8_F0F2);

fn lock() -> std::sync::MutexGuard<'static, Shared> {
    SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn hwnd(handle: u64) -> Result<HWND> {
    Ok(HWND(usize::try_from(handle)? as *mut _))
}

/// Show the panel, or update what it shows. The first call starts its thread.
pub fn show_dock(request: DockRequest) -> Result<()> {
    let mut shared = lock();
    if shared.request.as_ref() == Some(&request) && shared.window != 0 {
        return Ok(());
    }
    shared.request = Some(request);
    if shared.window != 0 {
        // SAFETY: Posting carries no pointers; a stale HWND is rejected by Win32.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(shared.window as *mut _)),
                WM_DOCK_UPDATE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        return Ok(());
    }
    if shared.thread.as_ref().is_some_and(|t| !t.is_finished()) {
        // Still starting: it applies the latest request once its window exists.
        return Ok(());
    }
    shared.thread = Some(thread::Builder::new().name("dock".into()).spawn(|| {
        let _ = run();
        let mut shared = lock();
        shared.window = 0;
        shared.layout = None;
    })?);
    Ok(())
}

/// Remove the panel and give its space back to the work area.
pub fn hide_dock() {
    let thread = {
        let mut shared = lock();
        shared.request = None;
        if shared.window != 0 {
            // SAFETY: Posting carries no pointers; a stale HWND is rejected by Win32.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(shared.window as *mut _)),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
        shared.thread.take()
    };
    if let Some(thread) = thread {
        let _ = thread.join();
    }
    let mut shared = lock();
    shared.layout = None;
    shared.clicks.clear();
}

/// The panel and slot rectangles last laid out, while the panel is shown.
pub fn dock_layout() -> Option<DockLayout> {
    lock().layout.clone()
}

/// Windows whose previews were clicked since the last call.
pub fn take_dock_clicks() -> Vec<u64> {
    std::mem::take(&mut lock().clicks)
}

/// Whether a dock panel exists in any process of this session.
pub fn dock_running() -> bool {
    // SAFETY: FindWindowW reads only the static class name.
    unsafe { FindWindowW(CLASS, PCWSTR::null()).is_ok_and(|w| !w.is_invalid()) }
}

/// The primary monitor's bounds and work area. With the panel shown the work
/// area already excludes it, so it is the arena.
pub fn dock_monitor() -> Result<(Rect, Rect)> {
    let _dpi = DpiContext::enter();
    // SAFETY: The output buffer is live, initialized and correctly sized.
    unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        ensure!(
            GetMonitorInfoW(monitor, &mut info).as_bool(),
            "Could not read the primary monitor"
        );
        Ok((from_rect(info.rcMonitor), from_rect(info.rcWork)))
    }
}

pub(super) fn is_docked(window: HWND) -> bool {
    // SAFETY: The properties are opaque markers, never dereferenced.
    unsafe {
        !GetPropW(window, DOCKED_PROP).is_invalid() || !GetPropW(window, PANEL_PROP).is_invalid()
    }
}

/// Mark or unmark a window as docked. Marked windows are invisible to tiling,
/// centered groups and directional focus, in every tt-pane process.
pub fn set_docked(handle: u64, docked: bool) -> Result<()> {
    let window = hwnd(handle)?;
    // SAFETY: The marker is an opaque non-null value, never dereferenced.
    unsafe {
        if docked {
            SetPropW(window, DOCKED_PROP, Some(HANDLE(std::ptr::dangling_mut())))?;
        } else {
            let _ = RemovePropW(window, DOCKED_PROP);
        }
    }
    Ok(())
}

/// The id and visible bounds of a window that still exists, docked or not.
pub fn docked_window(handle: u64) -> Option<(String, Rect)> {
    let id = super::window_identity(handle)?;
    let window = hwnd(handle).ok()?;
    let _dpi = DpiContext::enter();
    let (_, visible) = super::read_bounds(window).ok()?;
    Some((id, from_rect(visible)))
}

/// Move a window without the snapshot checks other moves use. Honors
/// `with_snap_animation` on the calling thread.
pub fn place_window(handle: u64, target: Rect) -> Result<Rect> {
    super::apply(Some(handle), &mut |_, _, _| Ok(target))
}

/// Every monitor's bounds, in physical pixels.
fn monitor_rects() -> Vec<Rect> {
    unsafe extern "system" fn collect(
        _monitor: HMONITOR,
        _dc: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: EnumDisplayMonitors passes a valid rect and our live Vec synchronously.
        unsafe { (*(data.0 as *mut Vec<Rect>)).push(from_rect(*rect)) };
        true.into()
    }
    let _dpi = DpiContext::enter();
    let mut rects = Vec::new();
    // SAFETY: The callback only borrows `rects` for the duration of this call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut rects as *mut _ as isize),
        );
    }
    rects
}

/// Where docked windows are parked at the preview's source size: mostly off
/// the dock's monitor at the panel's fill corner, so Windows still treats them
/// as on screen and keeps drawing them. Past every monitor if that corner
/// touches another display.
pub fn park_target(edge: DockEdge, fill: DockFill) -> Result<Rect> {
    let (monitor, _) = dock_monitor()?;
    let others: Vec<Rect> = monitor_rects()
        .into_iter()
        .filter(|rect| *rect != monitor)
        .collect();
    if let Some(rect) = dock::park_rect(monitor, edge, fill, &others) {
        return Ok(rect);
    }
    let _dpi = DpiContext::enter();
    // SAFETY: GetSystemMetrics takes no pointers.
    let (x, y) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) + 200,
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        )
    };
    Ok(Rect {
        x,
        y,
        width: dock::PREVIEW_SOURCE.0,
        height: dock::PREVIEW_SOURCE.1,
    })
}

/// Park a docked window at `park_target`.
pub fn park_window(handle: u64, edge: DockEdge, fill: DockFill) -> Result<Rect> {
    let target = park_target(edge, fill)?;
    crate::with_snap_animation(None, || place_window(handle, target))
}

/// Bring a window to the foreground and wait until Windows confirms it.
pub fn activate_handle(handle: u64) -> Result<()> {
    super::activate_window(hwnd(handle)?)
}

/// The window being moved or resized with the mouse right now, if any.
pub fn moving_window() -> Option<u64> {
    let mut gui = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: The output buffer is live, initialized and correctly sized.
    unsafe {
        GetGUIThreadInfo(0, &mut gui).ok()?;
    }
    (gui.flags & GUI_INMOVESIZE != GUITHREADINFO_FLAGS(0) && !gui.hwndMoveSize.is_invalid())
        .then_some(gui.hwndMoveSize.0 as usize as u64)
}

/// The mouse position in physical screen pixels.
pub fn cursor_position() -> Option<(i32, i32)> {
    let _dpi = DpiContext::enter();
    let mut point = POINT::default();
    // SAFETY: The output buffer is a live local.
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

struct Panel {
    thumbs: HashMap<u64, isize>,
    /// Registered with the shell as an AppBar, which reserves screen space.
    registered: bool,
    /// The rectangle last reserved with the shell.
    reserved: Option<Rect>,
    rect: Rect,
    slots: Vec<(u64, Rect)>,
    /// The empty slot shown while a window is dragged.
    drop: Option<Rect>,
    active: Option<u64>,
    highlight: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            thumbs: HashMap::new(),
            registered: false,
            reserved: None,
            rect: Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            slots: Vec::new(),
            drop: None,
            active: None,
            highlight: false,
        }
    }
}

thread_local! {
    static PANEL: RefCell<Panel> = RefCell::new(Panel::default());
}

fn run() -> Result<()> {
    let _dpi = DpiContext::enter();
    // SAFETY: The class and window use static strings and this module's window
    // procedure. The window, its AppBar and thumbnails live on this thread only.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_HAND)?,
            ..Default::default()
        };
        // Registering twice (a second panel after a hide) fails harmlessly.
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS,
            w!("tt-pane dock"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        SetPropW(window, PANEL_PROP, Some(HANDLE(std::ptr::dangling_mut())))?;
        lock().window = window.0 as usize;
        update(window);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn post(window: HWND, message: u32) {
    // SAFETY: Posting carries no pointers.
    unsafe {
        let _ = PostMessageW(Some(window), message, WPARAM(0), LPARAM(0));
    }
}

fn appbar(window: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: window,
        uCallbackMessage: WM_DOCK_APPBAR,
        ..Default::default()
    }
}

fn to_rect(rect: Rect) -> RECT {
    RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.width,
        bottom: rect.y + rect.height,
    }
}

fn edge_code(edge: DockEdge) -> u32 {
    match edge {
        DockEdge::Top => ABE_TOP,
        DockEdge::Left => ABE_LEFT,
        DockEdge::Right => ABE_RIGHT,
        DockEdge::Bottom => ABE_BOTTOM,
    }
}

/// The full-length strip along the edge, kept clear of the taskbar. A
/// registered AppBar asks the shell; otherwise the work area already excludes it.
fn edge_strip(window: HWND, edge: DockEdge, percent: u32, registered: bool) -> Result<Rect> {
    let (monitor, work) = dock_monitor()?;
    if !registered {
        return Ok(dock::strip_within(monitor, work, edge, percent));
    }
    let desired = dock::panel_rect(monitor, edge, percent);
    let thickness = if edge.horizontal() {
        desired.height
    } else {
        desired.width
    };
    let mut data = appbar(window);
    data.uEdge = edge_code(edge);
    data.rc = to_rect(desired);
    // SAFETY: The AppBar data is a live local with its size set; the shell
    // retains no pointer to it.
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &mut data);
    }
    // The shell may move the strip away from the taskbar; keep its thickness.
    match edge {
        DockEdge::Top => data.rc.bottom = data.rc.top + thickness,
        DockEdge::Bottom => data.rc.top = data.rc.bottom - thickness,
        DockEdge::Left => data.rc.right = data.rc.left + thickness,
        DockEdge::Right => data.rc.left = data.rc.right - thickness,
    }
    Ok(from_rect(data.rc))
}

/// Reserve `rect` with the shell, or give the space back with `None`. Windows
/// reserves the panel's thickness along the whole edge, however short it is.
fn reserve(window: HWND, panel: &mut Panel, edge: DockEdge, rect: Option<Rect>) {
    // SAFETY: The AppBar data is a live local with its size set; the shell
    // retains no pointer to it.
    unsafe {
        match rect {
            Some(rect) => {
                // Setting an unchanged position would notify every AppBar again.
                if panel.reserved == Some(rect) {
                    return;
                }
                let mut data = appbar(window);
                data.uEdge = edge_code(edge);
                data.rc = to_rect(rect);
                SHAppBarMessage(ABM_SETPOS, &mut data);
                panel.reserved = Some(rect);
            }
            None => {
                if panel.registered {
                    let mut data = appbar(window);
                    SHAppBarMessage(ABM_REMOVE, &mut data);
                }
                panel.registered = false;
                panel.reserved = None;
            }
        }
    }
}

/// Fit the source window's aspect ratio inside `slot`, centered.
fn fit(slot: Rect, source: SIZE) -> Rect {
    if source.cx <= 0 || source.cy <= 0 {
        return slot;
    }
    let (sw, sh) = (i64::from(source.cx), i64::from(source.cy));
    let (w, h) = (i64::from(slot.width), i64::from(slot.height));
    let (width, height) = if w * sh <= h * sw {
        (w, (w * sh / sw).max(1))
    } else {
        ((h * sw / sh).max(1), h)
    };
    Rect {
        x: slot.x + ((w - width) / 2) as i32,
        y: slot.y + ((h - height) / 2) as i32,
        width: width as i32,
        height: height as i32,
    }
}

fn update(window: HWND) {
    let Some(request) = lock().request.clone() else {
        // SAFETY: The window belongs to this thread.
        unsafe {
            let _ = DestroyWindow(window);
        }
        return;
    };
    // The shell can call back into this window while an AppBar message is in
    // flight; a re-entered update runs again once the current one finishes.
    PANEL.with(|cell| {
        let Ok(mut panel) = cell.try_borrow_mut() else {
            post(window, WM_DOCK_UPDATE);
            return;
        };
        let panel = &mut *panel;
        let docked = request.members.len();
        let count = docked + usize::from(request.dragging);
        // SAFETY: Thumbnails are registered to this thread's window and released
        // here or on destroy. Property structs are live locals.
        unsafe {
            panel.thumbs.retain(|handle, thumb| {
                let keep = request.members.contains(handle);
                if !keep {
                    let _ = DwmUnregisterThumbnail(*thumb);
                }
                keep
            });
            if count == 0 {
                // Nothing docked or dragged: hide and give the space back.
                reserve(window, panel, request.edge, None);
                let _ = ShowWindow(window, SW_HIDE);
                panel.slots.clear();
                panel.drop = None;
                lock().layout = None;
                return;
            }
            // Reserve space only once the docked windows fill the whole edge:
            // Windows reserves an AppBar's thickness along the entire edge, so a
            // shorter panel floats over its own corner instead. A drag never
            // reserves, so other windows don't move mid-drag.
            let Ok(mut strip) = edge_strip(window, request.edge, request.percent, panel.registered)
            else {
                return;
            };
            let full = docked > 0 && dock::fills(strip, request.edge, docked);
            if full != panel.registered {
                if full {
                    let mut data = appbar(window);
                    SHAppBarMessage(ABM_NEW, &mut data);
                    panel.registered = true;
                } else {
                    reserve(window, panel, request.edge, None);
                }
                let Ok(again) = edge_strip(window, request.edge, request.percent, panel.registered)
                else {
                    return;
                };
                strip = again;
            }
            if full {
                reserve(window, panel, request.edge, Some(strip));
            }
            panel.rect = dock::occupied(strip, request.edge, request.fill, count);
            let _ = SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                panel.rect.x,
                panel.rect.y,
                panel.rect.width,
                panel.rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let rects = dock::slots_from(panel.rect, request.edge, request.fill, count);
            panel.slots = request.members.iter().copied().zip(rects.clone()).collect();
            panel.drop = request
                .dragging
                .then(|| rects.get(docked).copied())
                .flatten();
            panel.active = request.active;
            panel.highlight = request.highlight;
            for &(handle, slot) in &panel.slots {
                let thumb = match panel.thumbs.get(&handle) {
                    Some(thumb) => *thumb,
                    None => match hwnd(handle).map(|source| DwmRegisterThumbnail(window, source)) {
                        Ok(Ok(thumb)) => {
                            panel.thumbs.insert(handle, thumb);
                            thumb
                        }
                        _ => continue,
                    },
                };
                let inner = crate::geometry::inset_rect(slot, 3, 3, 3, 3);
                let destination = fit(
                    inner,
                    DwmQueryThumbnailSourceSize(thumb).unwrap_or_default(),
                );
                let properties = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION
                        | DWM_TNP_VISIBLE
                        | DWM_TNP_OPACITY
                        | DWM_TNP_SOURCECLIENTAREAONLY,
                    rcDestination: to_rect(Rect {
                        x: destination.x - panel.rect.x,
                        y: destination.y - panel.rect.y,
                        ..destination
                    }),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                    ..Default::default()
                };
                let _ = DwmUpdateThumbnailProperties(thumb, &properties);
            }
            let _ = InvalidateRect(Some(window), None, true);
        }
        lock().layout = Some(DockLayout {
            panel: panel.rect,
            slots: panel.slots.clone(),
        });
    });
}

/// # Safety
/// Called by Win32 on the panel's thread with that thread's messages.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Every call below uses this thread's own window and live locals.
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONUP => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let _ = PANEL.with(|cell| {
                    cell.try_borrow().map(|panel| {
                        let point = (panel.rect.x + x, panel.rect.y + y);
                        let rects: Vec<Rect> = panel.slots.iter().map(|(_, r)| *r).collect();
                        if let Some(index) = dock::hit_test(&rects, point) {
                            lock().clicks.push(panel.slots[index].0);
                        }
                    })
                });
                LRESULT(0)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let _ = PANEL.with(|cell| cell.try_borrow().map(|panel| paint_panel(dc, &panel)));
                let _ = EndPaint(window, &paint);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_DOCK_UPDATE => {
                update(window);
                LRESULT(0)
            }
            WM_DOCK_APPBAR if wparam.0 as u32 == ABN_POSCHANGED => {
                post(window, WM_DOCK_REPOSITION);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                post(window, WM_DOCK_REPOSITION);
                DefWindowProcW(window, message, wparam, lparam)
            }
            WM_DOCK_REPOSITION => {
                // The taskbar or a display changed: forget the reservation so
                // the next update claims the strip again.
                match PANEL.with(|cell| cell.try_borrow_mut().map(|mut p| p.reserved = None)) {
                    Ok(()) => update(window),
                    Err(_) => post(window, WM_DOCK_REPOSITION),
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(window);
                LRESULT(0)
            }
            WM_DESTROY => {
                let registered = PANEL.with(|cell| {
                    cell.try_borrow_mut().map(|mut panel| {
                        for (_, thumb) in panel.thumbs.drain() {
                            let _ = DwmUnregisterThumbnail(thumb);
                        }
                        let registered = panel.registered;
                        *panel = Panel::default();
                        registered
                    })
                });
                // Removing an AppBar that was never added is harmless.
                if registered.unwrap_or(true) {
                    let mut data = appbar(window);
                    SHAppBarMessage(ABM_REMOVE, &mut data);
                }
                let _ = RemovePropW(window, PANEL_PROP);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// # Safety
/// `dc` is the panel's paint DC on its own thread.
unsafe fn paint_panel(dc: HDC, panel: &Panel) {
    let local = |rect: Rect| {
        to_rect(Rect {
            x: rect.x - panel.rect.x,
            y: rect.y - panel.rect.y,
            ..rect
        })
    };
    // SAFETY: Brushes are created and deleted here; rects are live locals.
    unsafe {
        let background = CreateSolidBrush(BACKGROUND);
        let slot = CreateSolidBrush(SLOT);
        let accent = CreateSolidBrush(ACCENT);
        let whole = RECT {
            left: 0,
            top: 0,
            right: panel.rect.width,
            bottom: panel.rect.height,
        };
        FillRect(dc, &whole, background);
        for &(handle, rect) in &panel.slots {
            let mut frame = local(rect);
            FillRect(dc, &frame, slot);
            if panel.active == Some(handle) {
                for _ in 0..3 {
                    FrameRect(dc, &frame, accent);
                    let _ = InflateRect(&mut frame, -1, -1);
                }
            }
        }
        if let Some(rect) = panel.drop {
            let mut frame = local(rect);
            let brush = if panel.highlight { accent } else { slot };
            for _ in 0..2 {
                FrameRect(dc, &frame, brush);
                let _ = InflateRect(&mut frame, -1, -1);
            }
        }
        if panel.highlight {
            let mut frame = whole;
            for _ in 0..4 {
                FrameRect(dc, &frame, accent);
                let _ = InflateRect(&mut frame, -1, -1);
            }
        }
        let _ = DeleteObject(background.into());
        let _ = DeleteObject(slot.into());
        let _ = DeleteObject(accent.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_the_source_aspect_ratio() {
        let slot = Rect {
            x: 10,
            y: 20,
            width: 320,
            height: 180,
        };
        assert_eq!(fit(slot, SIZE { cx: 1920, cy: 1080 }), slot);
        let tall = fit(slot, SIZE { cx: 1000, cy: 1000 });
        assert_eq!((tall.width, tall.height), (180, 180));
        assert_eq!(tall.x, 10 + 70);
        assert_eq!(fit(slot, SIZE::default()), slot);
    }
}
