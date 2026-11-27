//! The dock panel: a topmost AppBar on the primary monitor that shows live DWM
//! thumbnails of docked windows. Docked windows are parked at 1080p nearly off
//! screen, with a sliver under the panel so they keep drawing, and carry a property so every other
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
    /// Dock pick mode is on: number the first nine previews.
    pub numbered: bool,
    /// The window gaps, which the previews are laid out with.
    pub gaps: crate::geometry::SnapGaps,
}

/// Where the panel and its slots ended up, in physical screen pixels. There is
/// no layout while the panel is hidden, with nothing docked or dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockLayout {
    pub panel: Rect,
    pub slots: Vec<(u64, Rect)>,
    /// What the docked windows take of the edge, without a drag's drop slot:
    /// the whole strip once they fill it. `None` while nothing is docked.
    pub area: Option<Rect>,
}

struct Shared {
    request: Option<DockRequest>,
    layout: Option<DockLayout>,
    clicks: Vec<u64>,
    /// The window dock pick mode chose last, waiting for the next observation.
    picked: Option<u64>,
    /// Dock pick mode is on.
    picking: bool,
    window: usize,
    thread: Option<JoinHandle<()>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    request: None,
    layout: None,
    clicks: Vec::new(),
    picked: None,
    picking: false,
    window: 0,
    thread: None,
});

const CLASS: PCWSTR = w!("tt-pane.dock");
const OVERLAY_CLASS: PCWSTR = w!("tt-pane.dock-numbers");
const PANEL_PROP: PCWSTR = w!("tt-pane.dock-panel");
const DOCKED_PROP: PCWSTR = w!("tt-pane.docked");
const WM_DOCK_UPDATE: u32 = WM_APP + 0x40;
const WM_DOCK_APPBAR: u32 = WM_APP + 0x41;
const WM_DOCK_REPOSITION: u32 = WM_APP + 0x42;

/// The panel is a colour-keyed layered window: pixels painted in this colour
/// are see-through and let clicks fall through to the desktop. It is nearly
/// black so the anti-aliased edges of the pick-mode numbers blend into a dark
/// outline rather than a coloured fringe.
const TRANSPARENT_KEY: COLORREF = COLORREF(0x0001_0101);
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
    shared.picked = None;
}

/// The panel and slot rectangles last laid out, while the panel is shown.
pub fn dock_layout() -> Option<DockLayout> {
    lock().layout.clone()
}

/// Windows whose previews were clicked since the last call, then the one dock
/// pick mode chose, which counts as the latest click.
pub fn take_dock_clicks() -> Vec<u64> {
    let mut shared = lock();
    let mut clicks = std::mem::take(&mut shared.clicks);
    clicks.extend(shared.picked.take());
    clicks
}

/// Bring `handle` out of the dock at the next observation, as if its preview
/// were clicked. A newer pick replaces one still waiting, so stepping quickly
/// through the dock skips the windows in between.
pub fn pick_dock(handle: u64) {
    lock().picked = Some(handle);
}

/// The pick still waiting for the next observation.
pub fn pending_dock_pick() -> Option<u64> {
    lock().picked
}

/// Turn the numbered previews of dock pick mode on or off, right away.
pub fn set_dock_picking(on: bool) {
    let mut shared = lock();
    if shared.picking == on {
        return;
    }
    shared.picking = on;
    if let Some(request) = &mut shared.request {
        request.numbered = on;
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
        }
    }
}

/// Whether dock pick mode is on.
pub fn dock_picking() -> bool {
    lock().picking
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

/// Where a docked window of `size` is parked: mostly off the dock's monitor at
/// the panel's fill corner, with a sliver left on screen under the panel.
/// Chrome stops drawing a window that is on no screen at all, which freezes its
/// preview. If that corner touches another display, peeking into a gap between
/// monitors instead. Past every monitor when there is no such gap, or without
/// `live` previews, which saves the power those windows spend drawing. `step`
/// is the window's place in the dock, so docked windows don't park on top of
/// each other: Chrome stops drawing a window the others cover completely.
pub fn park_target(
    edge: DockEdge,
    fill: DockFill,
    size: (i32, i32),
    live: bool,
    step: usize,
) -> Result<Rect> {
    if live {
        let (monitor, _) = dock_monitor()?;
        let others: Vec<Rect> = monitor_rects()
            .into_iter()
            .filter(|rect| *rect != monitor)
            .collect();
        if let Some(rect) = dock::park_rect(monitor, edge, fill, size, step, &others) {
            return Ok(rect);
        }
        let monitors: Vec<Rect> = others.into_iter().chain([monitor]).collect();
        if let Some(rect) = dock::gap_park_rect(&monitors, size, step) {
            return Ok(rect);
        }
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
        width: size.0,
        height: size.1,
    })
}

/// How long a window gets to lay itself out at 1080p before it is parked.
/// Explorer needed about 150 ms to fill its file list.
const STAGING_SETTLE: std::time::Duration = std::time::Duration::from_millis(200);

/// Park a docked window. A window that isn't 1080p yet grows to it on the dock
/// monitor first and gets a moment to redraw, because some applications
/// (Explorer) leave content they lay out off screen blank. Moving an already
/// laid-out window off screen keeps its content.
pub fn park_window(
    handle: u64,
    edge: DockEdge,
    fill: DockFill,
    live: bool,
    step: usize,
) -> Result<Rect> {
    let target = park_target(edge, fill, dock::PREVIEW_SOURCE, live, step)?;
    crate::with_snap_animation(None, || {
        let current = docked_window(handle).map(|(_, bounds)| bounds);
        let sized = current.is_some_and(|bounds| {
            bounds.width.abs_diff(target.width) <= 2 && bounds.height.abs_diff(target.height) <= 2
        });
        if let Some(current) = current
            && !sized
        {
            let (_, work) = dock_monitor()?;
            place_window(handle, dock::staging_rect(current, work))?;
            std::thread::sleep(STAGING_SETTLE);
        }
        place_window(handle, target)
    })
}

/// Send a window in the arena back to the dock. It moves to the park spot at
/// its current size, which keeps its content (resizing it off screen would not).
/// With animation on (`with_snap_animation`) and a `slot`, a live thumbnail
/// stands in for it and glides into the slot on the same curve that brought
/// it out.
pub fn stow_window(
    handle: u64,
    slot: Option<Rect>,
    edge: DockEdge,
    fill: DockFill,
    live: bool,
    step: usize,
) -> Result<Rect> {
    let (_, bounds) =
        docked_window(handle).ok_or_else(|| anyhow::anyhow!("Window is no longer available"))?;
    let target = park_target(edge, fill, (bounds.width, bounds.height), live, step)?;
    let glide = match (slot, crate::snap_animation()) {
        (Some(slot), Some(duration)) => {
            Glide::new(handle, bounds).ok().map(|g| (g, slot, duration))
        }
        _ => None,
    };
    let result = crate::with_snap_animation(None, || place_window(handle, target));
    if let Some((glide, slot, duration)) = glide {
        let to = slot;
        glide.run(bounds, to, duration);
    }
    result
}

/// Bring a docked window from its preview `slot` to `target`. With animation
/// on, a live thumbnail of the parked window grows out of the slot and the real
/// window takes its place once, at full size, at the end. Growing the real
/// window from the slot instead makes it relayout on every frame, which
/// flickers, and stretches its preview in the panel as it goes.
pub fn lift_window(handle: u64, slot: Rect, target: Rect) -> Result<Rect> {
    let glide = crate::snap_animation().and_then(|duration| {
        docked_window(handle)?;
        let from = slot;
        Glide::new(handle, from)
            .ok()
            .map(|glide| (glide, from, duration))
    });
    let Some((glide, from, duration)) = glide else {
        let result = place_window(handle, target);
        refresh_now();
        return result;
    };
    glide.run(from, target, duration);
    let result = crate::with_snap_animation(None, || place_window(handle, target));
    // The window's preview is cropped for its docked size: recrop it for the
    // new size before the next frame composites, or it shows the wrong part of
    // the window, at the wrong size, until the panel catches up.
    refresh_now();
    // Let the real window composite at its new size before the stand-in goes.
    // SAFETY: DwmFlush takes no arguments.
    unsafe {
        let _ = DwmFlush();
        let _ = DwmFlush();
    }
    drop(glide);
    result
}

/// Lay the panel out again and wait for it, briefly, so previews are recropped
/// in the same frame as a docked window's resize.
fn refresh_now() {
    let window = lock().window;
    if window != 0 {
        // SAFETY: The message carries no pointers; a stale HWND is rejected by
        // Win32, and the timeout bounds the wait if the panel is busy.
        unsafe {
            let _ = SendMessageTimeoutW(
                HWND(window as *mut _),
                WM_DOCK_UPDATE,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                None,
            );
        }
    }
}

const GLIDE_CLASS: PCWSTR = w!("tt-pane.dock-glide");

/// # Safety
/// Called by Win32 with the glide window's messages.
unsafe extern "system" fn glide_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Never activate, and leave the client unpainted: the thumbnail covers it.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: Forwards this window's own message unchanged.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// A short-lived topmost window showing a live thumbnail of a docked window,
/// moved frame by frame on the calling thread.
struct Glide {
    window: HWND,
    thumb: isize,
    /// The docked window shown.
    source: HWND,
}

impl Glide {
    fn new(handle: u64, bounds: Rect) -> Result<Self> {
        let _dpi = DpiContext::enter();
        // SAFETY: Static class name and DefWindowProcW; the window and its
        // thumbnail are released by Drop on this same thread.
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(glide_procedure),
                hInstance: instance.into(),
                lpszClassName: GLIDE_CLASS,
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                GLIDE_CLASS,
                w!("tt-pane dock glide"),
                WS_POPUP,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let source = hwnd(handle)?;
            let mut glide = Self {
                window,
                thumb: 0,
                source,
            };
            SetPropW(window, PANEL_PROP, Some(HANDLE(std::ptr::dangling_mut())))?;
            glide.thumb = DwmRegisterThumbnail(window, source)?;
            glide.show(bounds)?;
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            // Composite the stand-in before the real window leaves.
            let _ = DwmFlush();
            Ok(glide)
        }
    }

    /// Show the glide at `rect`. The window is cropped to each frame's shape,
    /// as its preview is, so the glide starts exactly where the preview was
    /// and ends showing the whole window, with no jump at either end.
    fn show(&self, rect: Rect) -> Result<()> {
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_RECTSOURCE
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY,
            rcDestination: RECT {
                left: 0,
                top: 0,
                right: rect.width,
                bottom: rect.height,
            },
            rcSource: thumbnail_source(self.source, rect.width, rect.height),
            opacity: 255,
            fVisible: true.into(),
            ..Default::default()
        };
        // SAFETY: The thumbnail and window belong to this glide; the property
        // struct is a live local.
        unsafe {
            DwmUpdateThumbnailProperties(self.thumb, &properties)?;
            SetWindowPos(
                self.window,
                Some(HWND_TOPMOST),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE,
            )?;
        }
        Ok(())
    }

    /// Glide from `from` to `to` on the snap animation curve.
    fn run(&self, from: Rect, to: Rect, duration: std::time::Duration) {
        let _dpi = DpiContext::enter();
        crate::animation::run(
            from,
            to,
            duration,
            &mut |frame| self.show(frame),
            &mut || {
                // SAFETY: DwmFlush takes no arguments.
                if unsafe { DwmFlush() }.is_err() {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            },
        );
    }
}

impl Drop for Glide {
    fn drop(&mut self) {
        // SAFETY: Both handles were created by this glide on this thread.
        unsafe {
            if self.thumb != 0 {
                let _ = DwmUnregisterThumbnail(self.thumb);
            }
            let _ = DestroyWindow(self.window);
        }
    }
}

/// Bring a window to the foreground and wait until Windows confirms it.
/// Whether a window is minimized.
pub fn is_minimized(handle: u64) -> bool {
    // SAFETY: Win32 validates the opaque handle; no pointers are passed.
    hwnd(handle).is_ok_and(|window| unsafe { IsIconic(window).as_bool() })
}

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
    /// Where each thumbnail is drawn, in screen pixels. Painted opaque beneath
    /// the thumbnail so the preview takes clicks; the rest of the panel is
    /// transparent.
    previews: Vec<(u64, Rect)>,
    /// The empty slot shown while a window is dragged.
    drop: Option<Rect>,
    active: Option<u64>,
    highlight: bool,
    numbered: bool,
    /// A click-through window owned by the panel that draws the pick-mode
    /// numbers. Thumbnails cover anything the panel paints, so the numbers
    /// need a window of their own above them.
    overlay: HWND,
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
            previews: Vec::new(),
            drop: None,
            active: None,
            highlight: false,
            numbered: false,
            overlay: HWND::default(),
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
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
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
        SetLayeredWindowAttributes(window, TRANSPARENT_KEY, 0, LWA_COLORKEY)?;
        let overlay_class = WNDCLASSW {
            lpfnWndProc: Some(overlay_procedure),
            hInstance: instance.into(),
            lpszClassName: OVERLAY_CLASS,
            ..Default::default()
        };
        RegisterClassW(&overlay_class);
        // Owned by the panel, so it stays above it and goes away with it.
        let overlay = CreateWindowExW(
            WS_EX_TOOLWINDOW
                | WS_EX_TOPMOST
                | WS_EX_NOACTIVATE
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT,
            OVERLAY_CLASS,
            w!("tt-pane dock numbers"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )?;
        SetLayeredWindowAttributes(overlay, TRANSPARENT_KEY, 0, LWA_COLORKEY)?;
        PANEL.with(|cell| cell.borrow_mut().overlay = overlay);
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


/// The part of `source` a thumbnail drawn at `width` × `height` shows, relative
/// to the source window's own rectangle: its visible frame, without the
/// invisible resize borders, cropped to that shape around the middle.
fn thumbnail_source(source: HWND, width: i32, height: i32) -> RECT {
    let mut bounds = RECT::default();
    let mut visible = RECT::default();
    // SAFETY: Both out-params are live locals of the right size; Win32 rejects
    // a stale handle.
    let (bounds_ok, visible_ok) = unsafe {
        (
            GetWindowRect(source, &mut bounds).is_ok(),
            DwmGetWindowAttribute(
                source,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut visible as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            )
            .is_ok(),
        )
    };
    let whole = Rect {
        x: 0,
        y: 0,
        width: bounds.right - bounds.left,
        height: bounds.bottom - bounds.top,
    };
    let shown = if bounds_ok && visible_ok {
        let inside = Rect {
            x: visible.left - bounds.left,
            y: visible.top - bounds.top,
            width: visible.right - visible.left,
            height: visible.bottom - visible.top,
        };
        if inside.width > 0 && inside.height > 0 {
            inside
        } else {
            whole
        }
    } else {
        whole
    };
    to_rect(dock::crop_to_aspect(shown, width, height))
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
        let spacing = dock::Spacing::from_gaps(request.gaps);
        let count = docked + usize::from(request.dragging);
        let mut area;
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
                panel.previews.clear();
                panel.drop = None;
                place_overlay(panel);
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
            let full = docked > 0 && dock::fills(strip, request.edge, docked, spacing);
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
            area = (docked > 0).then(|| {
                if full {
                    strip
                } else {
                    dock::occupied(strip, request.edge, request.fill, docked, spacing)
                }
            });
            panel.rect = dock::occupied(strip, request.edge, request.fill, count, spacing);
            let _ = SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                panel.rect.x,
                panel.rect.y,
                panel.rect.width,
                panel.rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let rects = dock::slots_from(panel.rect, request.edge, request.fill, count, spacing);
            panel.slots = request.members.iter().copied().zip(rects.clone()).collect();
            panel.drop = request
                .dragging
                .then(|| rects.get(docked).copied())
                .flatten();
            panel.active = request.active;
            panel.highlight = request.highlight;
            panel.numbered = request.numbered;
            panel.previews.clear();
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
                let destination = slot;
                let Ok(source) = hwnd(handle) else {
                    continue;
                };
                let properties = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION
                        | DWM_TNP_RECTSOURCE
                        | DWM_TNP_VISIBLE
                        | DWM_TNP_OPACITY
                        | DWM_TNP_SOURCECLIENTAREAONLY,
                    rcDestination: to_rect(Rect {
                        x: destination.x - panel.rect.x,
                        y: destination.y - panel.rect.y,
                        ..destination
                    }),
                    rcSource: thumbnail_source(source, destination.width, destination.height),
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                };
                let _ = DwmUpdateThumbnailProperties(thumb, &properties);
                panel.previews.push((handle, destination));
            }
            // Snaps keep clear of what the previews show, not the empty space
            // a wide window leaves in its slot. A full edge is reserved with
            // the shell as a whole strip, so it stays as it is.
            if !full {
                let shown: Vec<Rect> = panel.previews.iter().map(|&(_, rect)| rect).collect();
                area = area.map(|area| dock::shrink_to(area, request.edge, &shown));
            }
            let _ = InvalidateRect(Some(window), None, true);
            place_overlay(panel);
        }
        lock().layout = Some(DockLayout {
            panel: panel.rect,
            slots: panel.slots.clone(),
            area,
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

/// The height of a pick-mode number centred on a preview.
fn number_height(slot: Rect) -> i32 {
    (slot.height * 2 / 5).clamp(18, 120)
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
        let background = CreateSolidBrush(TRANSPARENT_KEY);
        let slot = CreateSolidBrush(SLOT);
        let accent = CreateSolidBrush(ACCENT);
        let whole = RECT {
            left: 0,
            top: 0,
            right: panel.rect.width,
            bottom: panel.rect.height,
        };
        FillRect(dc, &whole, background);
        // Hidden under the thumbnails, but keeps those pixels clickable.
        // The active preview's outline is drawn over it by the overlay.
        for &(_, preview) in &panel.previews {
            FillRect(dc, &local(preview), slot);
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

/// The preview of the window that's out in the arena, if it has one.
fn active_preview(panel: &Panel) -> Option<Rect> {
    let active = panel.active?;
    panel
        .previews
        .iter()
        .find(|(handle, _)| *handle == active)
        .map(|&(_, rect)| rect)
}

/// Places the overlay over the panel while it has something to draw (pick-mode
/// numbers or the active preview's outline), and hides it otherwise.
///
/// # Safety
/// Called on the panel's thread, which owns the overlay.
unsafe fn place_overlay(panel: &Panel) {
    if panel.overlay.is_invalid() {
        return;
    }
    // SAFETY: The overlay belongs to this thread.
    unsafe {
        let numbers = panel.numbered && !panel.slots.is_empty();
        if numbers || active_preview(panel).is_some() {
            let _ = SetWindowPos(
                panel.overlay,
                Some(HWND_TOPMOST),
                panel.rect.x,
                panel.rect.y,
                panel.rect.width,
                panel.rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(panel.overlay), None, true);
        } else {
            let _ = ShowWindow(panel.overlay, SW_HIDE);
        }
    }
}

/// # Safety
/// Called by Win32 on the panel's thread with the overlay's messages.
unsafe extern "system" fn overlay_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Every call below uses this thread's own window and live locals.
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let _ = PANEL.with(|cell| cell.try_borrow().map(|panel| paint_numbers(dc, &panel)));
                let _ = EndPaint(window, &paint);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// Paints the overlay over a transparent background: the active preview's
/// outline, inside its edges so the preview keeps its exact size, and in pick
/// mode white numbers outlined in black, centred on the first nine previews.
///
/// # Safety
/// `dc` is the overlay's paint DC on the panel's thread.
unsafe fn paint_numbers(dc: HDC, panel: &Panel) {
    // SAFETY: GDI objects are selected into this DC and deleted before
    // returning; the text and rects are live locals.
    unsafe {
        let clear = CreateSolidBrush(TRANSPARENT_KEY);
        let whole = RECT {
            left: 0,
            top: 0,
            right: panel.rect.width,
            bottom: panel.rect.height,
        };
        FillRect(dc, &whole, clear);
        let _ = DeleteObject(clear.into());
        if let Some(rect) = active_preview(panel) {
            let accent = CreateSolidBrush(ACCENT);
            let mut frame = to_rect(Rect {
                x: rect.x - panel.rect.x,
                y: rect.y - panel.rect.y,
                ..rect
            });
            for _ in 0..3 {
                FrameRect(dc, &frame, accent);
                let _ = InflateRect(&mut frame, -1, -1);
            }
            let _ = DeleteObject(accent.into());
        }
        if !panel.numbered {
            return;
        }
        let Some(&(_, first)) = panel.slots.first() else {
            return;
        };
        let height = number_height(first);
        let font = CreateFontW(
            height,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let pen = CreatePen(PS_SOLID, (height / 10).max(3), COLORREF(0x0000_0000));
        let fill = CreateSolidBrush(COLORREF(0x00FF_FFFF));
        let previous_font = SelectObject(dc, font.into());
        let previous_pen = SelectObject(dc, pen.into());
        let previous_brush = SelectObject(dc, fill.into());
        SetBkMode(dc, TRANSPARENT);
        for (index, &(handle, slot)) in panel.slots.iter().take(9).enumerate() {
            // Centred on the preview, which may not fill its slot.
            let slot = panel
                .previews
                .iter()
                .find(|(shown, _)| *shown == handle)
                .map_or(slot, |&(_, rect)| rect);
            let text: Vec<u16> = (index + 1).to_string().encode_utf16().collect();
            let mut size = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &text, &mut size);
            let x = slot.x - panel.rect.x + (slot.width - size.cx) / 2;
            let y = slot.y - panel.rect.y + (slot.height - size.cy) / 2;
            // Stroke the outline first, then fill the same outline on top, so
            // the black stroke sits outside the white face.
            for stroke in [true, false] {
                let _ = BeginPath(dc);
                let _ = TextOutW(dc, x, y, &text);
                let _ = EndPath(dc);
                if stroke {
                    let _ = StrokePath(dc);
                } else {
                    let _ = FillPath(dc);
                }
            }
        }
        SelectObject(dc, previous_font);
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(fill.into());
    }
}

