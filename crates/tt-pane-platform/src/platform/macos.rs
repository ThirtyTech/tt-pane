use crate::geometry::{Action, Rect, monitor_for};
use anyhow::{Context, Result, ensure};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeID, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use core_graphics::geometry::{CGPoint, CGSize};
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSScreen,
    NSWorkspace,
};
use objc2_foundation::NSString;
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
// SAFETY: These declarations match ApplicationServices AXUIElement.h/AXValue.h.
// Core Foundation Boolean is an unsigned byte; AXValueType is a 32-bit enum.
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, name: CFStringRef, value: CFTypeRef)
    -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut c_void) -> u8;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFStringRef) -> i32;
    fn AXUIElementIsAttributeSettable(
        element: CFTypeRef,
        name: CFStringRef,
        settable: *mut u8,
    ) -> i32;
    // Private but stable for many releases; window managers such as Rectangle,
    // Amethyst and yabai use it to pair AX windows with CGWindowIDs.
    fn _AXUIElementGetWindow(element: CFTypeRef, window: *mut u32) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
// SAFETY: Matches CGEventSource.h: a 32-bit state ID and mouse button, C bool result.
unsafe extern "C" {
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

// SAFETY: Matches <dlfcn.h>; only reads the NUL-terminated symbol name.
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

fn attribute(element: &CFType, key: &str) -> Result<CFType> {
    ensure_element(element)?;
    let mut value = std::ptr::null();
    // SAFETY: The element type was checked; the CFString and output slot remain
    // live for the call. Success returns an owned (+1) Core Foundation object.
    let result = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            CFString::new(key).as_concrete_TypeRef(),
            &mut value,
        )
    };
    ensure!(
        result == 0 && !value.is_null(),
        "Cannot read {key} (Accessibility error {result})"
    );
    // SAFETY: Copy returned a non-null owned reference; CFType now releases it.
    Ok(unsafe { CFType::wrap_under_create_rule(value) })
}

const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;

fn ensure_element(element: &CFType) -> Result<()> {
    // SAFETY: Type-ID queries take no pointers and have no preconditions.
    let expected = unsafe { AXUIElementGetTypeID() };
    ensure!(element.type_of() == expected, "Expected AXUIElement");
    Ok(())
}

fn ensure_value(value: &CFType) -> Result<()> {
    // SAFETY: Type-ID queries take no pointers and have no preconditions.
    let expected = unsafe { AXValueGetTypeID() };
    ensure!(value.type_of() == expected, "Expected AXValue");
    Ok(())
}

fn position_value(point: &CGPoint) -> Result<CFType> {
    // SAFETY: AX_POINT requires a CGPoint; this typed reference guarantees its
    // layout, alignment and lifetime. AXValueCreate copies it synchronously.
    let raw = unsafe { AXValueCreate(AX_POINT, (point as *const CGPoint).cast()) };
    ensure!(!raw.is_null(), "Could not create Accessibility position");
    // SAFETY: Create returned a non-null owned (+1) object, adopted exactly once.
    Ok(unsafe { CFType::wrap_under_create_rule(raw) })
}

fn size_value(size: &CGSize) -> Result<CFType> {
    // SAFETY: AX_SIZE requires a CGSize; this typed reference guarantees its
    // layout, alignment and lifetime. AXValueCreate copies it synchronously.
    let raw = unsafe { AXValueCreate(AX_SIZE, (size as *const CGSize).cast()) };
    ensure!(!raw.is_null(), "Could not create Accessibility size");
    // SAFETY: Create returned a non-null owned (+1) object, adopted exactly once.
    Ok(unsafe { CFType::wrap_under_create_rule(raw) })
}

fn read_position(value: &CFType) -> Result<CGPoint> {
    ensure_value(value)?;
    let mut point = CGPoint::new(0.0, 0.0);
    // SAFETY: value is a checked AXValue. The initialized output is exactly the
    // CGPoint requested by AX_POINT; a different encoded type returns false.
    let success = unsafe {
        AXValueGetValue(
            value.as_CFTypeRef(),
            AX_POINT,
            (&mut point as *mut CGPoint).cast(),
        )
    };
    ensure!(success != 0, "Invalid AX position");
    Ok(point)
}

fn read_size(value: &CFType) -> Result<CGSize> {
    ensure_value(value)?;
    let mut size = CGSize::new(0.0, 0.0);
    // SAFETY: value is a checked AXValue. The initialized output is exactly the
    // CGSize requested by AX_SIZE; a different encoded type returns false.
    let success = unsafe {
        AXValueGetValue(
            value.as_CFTypeRef(),
            AX_SIZE,
            (&mut size as *mut CGSize).cast(),
        )
    };
    ensure!(success != 0, "Invalid AX size");
    Ok(size)
}

fn set_attribute(window: &CFType, key: &str, value: &CFType) -> Result<()> {
    ensure_element(window)?;
    // SAFETY: window is a checked AXUIElement. Both CF objects and the attribute
    // string remain alive during the synchronous call; none transfers ownership.
    let result = unsafe {
        AXUIElementSetAttributeValue(
            window.as_CFTypeRef(),
            CFString::new(key).as_concrete_TypeRef(),
            value.as_CFTypeRef(),
        )
    };
    ensure!(
        result == 0,
        "Cannot set {key} (Accessibility error {result}); the window may not be resizable"
    );
    Ok(())
}

fn set_position(window: &CFType, point: &CGPoint) -> Result<()> {
    set_attribute(window, "AXPosition", &position_value(point)?)
}

fn set_size(window: &CFType, size: &CGSize) -> Result<()> {
    set_attribute(window, "AXSize", &size_value(size)?)
}

/// Whether tt-pane may control other applications' windows. With `prompt`,
/// an untrusted process also asks macOS to show its Accessibility dialog,
/// which adds tt-pane to the list in System Settings and offers to open it.
/// macOS shows that dialog at most once per launch.
pub fn accessibility_trusted(prompt: bool) -> bool {
    if !prompt {
        // SAFETY: This process-level query takes no arguments or pointers.
        return unsafe { AXIsProcessTrusted() } != 0;
    }
    // The value of kAXTrustedCheckOptionPrompt, a constant CFString.
    let options = CFDictionary::from_CFType_pairs(&[(
        CFString::from_static_string("AXTrustedCheckOptionPrompt"),
        CFBoolean::true_value(),
    )]);
    // SAFETY: The dictionary is a live CFDictionary for the duration of the
    // call; the function only reads it and retains nothing.
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

/// Show a modal alert. tt-pane.app has no terminal, so this is how a launch
/// from Finder reports an error that would otherwise only reach stderr.
pub fn show_alert(title: &str, message: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.activate();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

fn read_rect(window: &CFType) -> Result<Rect> {
    let point = read_position(&attribute(window, "AXPosition")?)?;
    let size = read_size(&attribute(window, "AXSize")?)?;
    Ok(Rect {
        x: point.x.round() as i32,
        y: point.y.round() as i32,
        width: size.width.round() as i32,
        height: size.height.round() as i32,
    })
}

fn application_element(pid: i32) -> Result<CFType> {
    // SAFETY: AX validates the PID; this returns an owned reference, with no
    // Rust pointers retained.
    let raw = unsafe { AXUIElementCreateApplication(pid) };
    ensure!(!raw.is_null(), "Could not access application {pid}");
    // SAFETY: Create returned a non-null owned (+1) CF object, adopted once.
    Ok(unsafe { CFType::wrap_under_create_rule(raw) })
}

fn flag(element: &CFType, key: &str) -> bool {
    attribute(element, key)
        .ok()
        .and_then(|value| value.downcast::<CFBoolean>())
        .is_some_and(bool::from)
}

/// Standard Accessibility windows of an application, minimized ones included.
fn standard_windows(pid: i32) -> Vec<CFType> {
    let Ok(app) = application_element(pid) else {
        return Vec::new();
    };
    let Some(windows) = attribute(&app, "AXWindows")
        .ok()
        .and_then(|value| value.downcast::<CFArray>())
    else {
        return Vec::new();
    };
    windows
        .iter()
        .filter_map(|item| {
            // SAFETY: AXWindows holds live CF objects retained by the array.
            let window = unsafe { CFType::wrap_under_get_rule(*item) };
            attribute(&window, "AXSubrole")
                .ok()
                .and_then(|value| value.downcast::<CFString>())
                .is_some_and(|role| role == "AXStandardWindow")
                .then_some(window)
        })
        .collect()
}

/// Standard, non-minimized Accessibility windows of an application.
fn application_windows(pid: i32) -> Vec<(CFType, Rect)> {
    standard_windows(pid)
        .into_iter()
        .filter(|window| !flag(window, "AXMinimized"))
        .filter_map(|window| {
            let rect = read_rect(&window).ok()?;
            Some((window, rect))
        })
        .collect()
}

/// The CGWindowID behind an Accessibility window.
fn window_number(window: &CFType) -> Option<u32> {
    ensure_element(window).ok()?;
    let mut number = 0;
    // SAFETY: window is a checked AXUIElement; the output slot is a live u32.
    let result = unsafe { _AXUIElementGetWindow(window.as_CFTypeRef(), &mut number) };
    (result == 0 && number != 0).then_some(number)
}

fn settable(window: &CFType, key: &str) -> bool {
    if ensure_element(window).is_err() {
        return false;
    }
    let mut value = 0;
    // SAFETY: window is a checked AXUIElement; the string and output outlive the call.
    let result = unsafe {
        AXUIElementIsAttributeSettable(
            window.as_CFTypeRef(),
            CFString::new(key).as_concrete_TypeRef(),
            &mut value,
        )
    };
    result == 0 && value != 0
}

struct ScreenInfo {
    frames: Vec<Rect>,
    work_areas: Vec<Rect>,
}

impl ScreenInfo {
    fn read(mtm: MainThreadMarker) -> Result<Self> {
        let screens = NSScreen::screens(mtm);
        let primary = screens.firstObject().context("No screens found")?;
        let frame = primary.frame();
        let primary_top = frame.origin.y + frame.size.height;
        // Cocoa uses bottom-left origins; Accessibility uses the primary screen's top-left.
        let convert = |r: objc2_foundation::NSRect| Rect {
            x: r.origin.x.round() as i32,
            y: (primary_top - r.origin.y - r.size.height).round() as i32,
            width: r.size.width.round() as i32,
            height: r.size.height.round() as i32,
        };
        Ok(Self {
            frames: screens.iter().map(|s| convert(s.frame())).collect(),
            work_areas: screens.iter().map(|s| convert(s.visibleFrame())).collect(),
        })
    }

    fn work_area(&self, window: Rect) -> Option<Rect> {
        monitor_for(window, &self.frames).map(|index| self.work_areas[index])
    }
}

// CoreGraphics and Accessibility bounds can differ by rounding.
fn near(a: Rect, b: Rect) -> bool {
    [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
        .into_iter()
        .all(|d| d.abs() <= 2)
}

struct ListedWindow {
    number: u32,
    pid: i32,
    layer: i64,
    rect: Rect,
}

fn number(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
    dict.find(CFString::new(key))?
        .downcast::<CFNumber>()?
        .to_f64()
}

/// On-screen windows on the current Space, front to back, in Accessibility coordinates.
fn listed_windows() -> Result<Vec<ListedWindow>> {
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly,
    };
    let list = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )
    .context("Could not list on-screen windows")?;
    Ok(list
        .iter()
        .filter_map(|item| {
            // SAFETY: The window list holds live CFDictionaries retained by the array.
            let info: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule((*item).cast()) };
            let bounds = info
                .find(CFString::new("kCGWindowBounds"))?
                .downcast::<CFDictionary>()?;
            // SAFETY: kCGWindowBounds is a CFDictionary of CFString keys to CFNumbers.
            let bounds: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(bounds.as_concrete_TypeRef()) };
            if number(&info, "kCGWindowAlpha").is_some_and(|alpha| alpha <= 0.0) {
                return None;
            }
            Some(ListedWindow {
                number: number(&info, "kCGWindowNumber")? as u32,
                pid: number(&info, "kCGWindowOwnerPID")? as i32,
                layer: number(&info, "kCGWindowLayer")? as i64,
                rect: Rect {
                    x: number(&bounds, "X")?.round() as i32,
                    y: number(&bounds, "Y")?.round() as i32,
                    width: number(&bounds, "Width")?.round() as i32,
                    height: number(&bounds, "Height")?.round() as i32,
                },
            })
        })
        .collect())
}

/// Focus a spatial neighbor of the frontmost application's focused window.
pub fn focus_window(action: Action, visibility: super::FocusVisibility) -> Result<()> {
    use super::FocusVisibility;
    ensure!(
        accessibility_trusted(false),
        "Grant tt-pane Accessibility access in System Settings → Privacy & Security → Accessibility"
    );
    let mtm =
        MainThreadMarker::new().context("Window control must run on the macOS main thread")?;
    let front = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .context("No frontmost application")?;
    let front_pid = front.processIdentifier();
    let selected = attribute(&application_element(front_pid)?, "AXFocusedWindow")
        .context("Select an application window to change focus")?;
    let current = read_rect(&selected)?;
    let screens = ScreenInfo::read(mtm)?;
    // Hold enters middle windows from an edge, or exits a middle window to the edges.
    let hold = visibility == FocusVisibility::Hold;
    let snapped_only = hold && !crate::visibility::is_snapped(current, screens.work_area(current));
    let middle_only = hold && !snapped_only;
    let visibility = if hold {
        FocusVisibility::IncludeHidden
    } else {
        visibility
    };
    let own_pid = std::process::id() as i32;
    let mut selected_seen = false;
    let mut ax_windows = std::collections::HashMap::<i32, Vec<(CFType, Rect)>>::new();
    let mut above = Vec::new();
    let mut candidates = Vec::new();
    let mut bounds = Vec::new();
    // Windows at or above the Dock layer (menu bar, Dock, overlays) are not app windows.
    for listed in listed_windows()?
        .into_iter()
        .filter(|w| (0..20).contains(&w.layer))
    {
        let is_selected = !selected_seen && listed.pid == front_pid && near(listed.rect, current);
        selected_seen |= is_selected;
        if !is_selected && listed.layer == 0 && listed.pid != own_pid {
            let work_area = screens.work_area(listed.rect);
            let snapped = crate::visibility::is_snapped(listed.rect, work_area);
            let element = ax_windows
                .entry(listed.pid)
                .or_insert_with(|| application_windows(listed.pid))
                .iter()
                .position(|(_, rect)| near(*rect, listed.rect));
            if let Some(element) = element
                && (!snapped_only || snapped)
                && (!middle_only || !snapped)
                && crate::visibility::focus_eligible(
                    listed.rect,
                    work_area,
                    &screens.frames,
                    &above,
                    visibility,
                )
            {
                let (window, _) = ax_windows
                    .get_mut(&listed.pid)
                    .unwrap()
                    .swap_remove(element);
                candidates.push((listed.pid, window));
                bounds.push(listed.rect);
            }
        }
        // Every window, including the selected one, covers the windows beneath it.
        if visibility != FocusVisibility::IncludeHidden {
            above.push(listed.rect);
        }
    }
    let Some(index) = crate::geometry::focus_neighbor(current, &bounds, action) else {
        return Ok(());
    };
    let (pid, window) = &candidates[index];
    activate_window(*pid, window)
}

fn activate_window(pid: i32, window: &CFType) -> Result<()> {
    // SAFETY: window is a live AXUIElement and the action string outlives the call.
    let result = unsafe {
        AXUIElementPerformAction(
            window.as_CFTypeRef(),
            CFString::from_static_string("AXRaise").as_concrete_TypeRef(),
        )
    };
    ensure!(
        result == 0,
        "Cannot raise the window (Accessibility error {result})"
    );
    let _ = set_attribute(window, "AXMain", &CFBoolean::true_value().as_CFType());
    // Accessibility can bring a background application forward where cooperative
    // NSRunningApplication activation is refused; fall back to it otherwise.
    let app = application_element(pid)?;
    if set_attribute(&app, "AXFrontmost", &CFBoolean::true_value().as_CFType()).is_err() {
        let running = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .context("The window's application has quit")?;
        ensure!(
            running.activateWithOptions(NSApplicationActivationOptions::empty()),
            "macOS refused to activate the window's application"
        );
    }
    Ok(())
}

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    ensure!(
        accessibility_trusted(false),
        "Grant tt-pane Accessibility access in System Settings → Privacy & Security → Accessibility"
    );
    let mtm =
        MainThreadMarker::new().context("Window control must run on the macOS main thread")?;
    let application = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .context("No frontmost application")?;
    // SAFETY: The PID comes from a live NSRunningApplication; this returns an
    // owned reference, with no Rust pointers retained.
    let raw = unsafe { AXUIElementCreateApplication(application.processIdentifier()) };
    ensure!(!raw.is_null(), "Could not access frontmost application");
    // SAFETY: Create returned a non-null owned (+1) CF object, adopted once.
    let app = unsafe { CFType::wrap_under_create_rule(raw) };
    let window = attribute(&app, "AXFocusedWindow")?;
    let position = attribute(&window, "AXPosition")?;
    let size = attribute(&window, "AXSize")?;
    let point = read_position(&position)?;
    let dimensions = read_size(&size)?;
    let current = Rect {
        x: point.x.round() as i32,
        y: point.y.round() as i32,
        width: dimensions.width.round() as i32,
        height: dimensions.height.round() as i32,
    };
    let screens = NSScreen::screens(mtm);
    let primary = screens.firstObject().context("No screens found")?;
    let frame = primary.frame();
    let primary_top = frame.origin.y + frame.size.height;
    // Cocoa uses bottom-left origins; Accessibility uses the primary screen's top-left.
    let convert = |r: objc2_foundation::NSRect| Rect {
        x: r.origin.x.round() as i32,
        y: (primary_top - r.origin.y - r.size.height).round() as i32,
        width: r.size.width.round() as i32,
        height: r.size.height.round() as i32,
    };
    let frames: Vec<Rect> = screens
        .iter()
        .map(|screen| convert(screen.frame()))
        .collect();
    let index = monitor_for(current, &frames).context("No monitor found")?;
    let area = convert(screens.objectAtIndex(index).visibleFrame());
    // Include the application PID with the native AX element's identity hash.
    // SAFETY: window owns a live Core Foundation reference for this synchronous query.
    let identity = unsafe { core_foundation::base::CFHash(window.as_CFTypeRef()) };
    let id = format!("ax:{}:{identity}", application.processIdentifier());
    let target = resolve(&id, area, current)?;
    if let Some(duration) = crate::snap_animation() {
        let mut last = current;
        crate::animation::run(
            current,
            target,
            duration,
            &mut |frame| {
                let size = CGSize::new(frame.width as f64, frame.height as f64);
                let point = CGPoint::new(frame.x as f64, frame.y as f64);
                // Shrink before moving and move before growing, so no frame
                // pushes the window past the screen edge it travels along.
                if frame.width < last.width || frame.height < last.height {
                    set_size(&window, &size)?;
                    set_position(&window, &point)?;
                } else {
                    set_position(&window, &point)?;
                    set_size(&window, &size)?;
                }
                last = frame;
                Ok(())
            },
            &mut crate::animation::pause,
        );
    }
    let size = CGSize::new(target.width as f64, target.height as f64);
    let point = CGPoint::new(target.x as f64, target.y as f64);
    set_size(&window, &size)?;
    set_position(&window, &point)?;
    // Some applications constrain the first resize against the previous monitor.
    set_size(&window, &size)?;
    let actual_position = read_position(&attribute(&window, "AXPosition")?)?;
    let actual_size = read_size(&attribute(&window, "AXSize")?)?;
    let actual = Rect {
        x: actual_position.x.round() as i32,
        y: actual_position.y.round() as i32,
        width: actual_size.width.round() as i32,
        height: actual_size.height.round() as i32,
    };
    let aligned = current.align_width_result(target, actual);
    if aligned.x != actual.x {
        set_position(&window, &CGPoint::new(aligned.x as f64, actual.y as f64))?;
        let point = read_position(&attribute(&window, "AXPosition")?)?;
        return Ok(Rect {
            x: point.x.round() as i32,
            y: point.y.round() as i32,
            ..actual
        });
    }
    Ok(actual)
}

/// A restored standard window, identified by its CGWindowID (stable for the
/// window's lifetime) and owning process.
#[derive(Debug, Clone)]
pub struct WindowSnapshot {
    pub handle: u64,
    pub id: String,
    pub area: Rect,
    pub bounds: Rect,
}

fn window_id(pid: i32, number: u32) -> String {
    format!("ax:{pid}:{number}")
}

fn trusted() -> Result<()> {
    ensure!(
        accessibility_trusted(false),
        "Grant tt-pane Accessibility access in System Settings → Privacy & Security → Accessibility"
    );
    Ok(())
}

fn main_thread() -> Result<MainThreadMarker> {
    MainThreadMarker::new().context("Window control must run on the macOS main thread")
}

/// Owner of an on-screen window; minimized windows and other Spaces report none.
fn owner(number: u32) -> Option<i32> {
    use core_graphics::window::{copy_window_info, kCGWindowListOptionIncludingWindow};
    let list = copy_window_info(kCGWindowListOptionIncludingWindow, number)?;
    let item = list.iter().next()?;
    // SAFETY: The window list holds live CFDictionaries retained by the array.
    let info: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule((*item).cast()) };
    let onscreen = info
        .find(CFString::new("kCGWindowIsOnscreen"))
        .and_then(|value| value.downcast::<CFBoolean>())
        .is_some_and(bool::from);
    if !onscreen {
        return None;
    }
    number_value(&info, "kCGWindowOwnerPID")
}

fn number_value(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<i32> {
    number(dict, key).map(|value| value as i32)
}

/// The live AX window and its current frame, if it is still a restored standard window.
fn find_window(handle: u64) -> Option<(i32, CFType, Rect)> {
    let number = u32::try_from(handle).ok()?;
    let pid = owner(number)?;
    application_windows(pid)
        .into_iter()
        .find(|(window, _)| window_number(window) == Some(number))
        .map(|(window, rect)| (pid, window, rect))
}

pub fn foreground_handle() -> Option<u64> {
    if !accessibility_trusted(false) {
        return None;
    }
    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let app = application_element(front.processIdentifier()).ok()?;
    window_number(&attribute(&app, "AXFocusedWindow").ok()?).map(u64::from)
}

/// Read without restoring, activating, or repositioning a window. Minimized
/// windows and windows on other Spaces are excluded.
pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    trusted()?;
    let screens = ScreenInfo::read(main_thread()?)?;
    let Some((pid, _, bounds)) = find_window(handle) else {
        return Ok(None);
    };
    Ok(screens.work_area(bounds).map(|area| WindowSnapshot {
        handle,
        id: window_id(pid, handle as u32),
        area,
        bounds,
    }))
}

/// Revalidate immediately before a move, never overwriting a manual move made
/// since the coordinator's last observation.
fn revalidate(expected: &WindowSnapshot) -> Result<(i32, CFType)> {
    trusted()?;
    let screens = ScreenInfo::read(main_thread()?)?;
    let (pid, window, bounds) =
        find_window(expected.handle).context("Window is no longer available")?;
    ensure!(
        window_id(pid, expected.handle as u32) == expected.id
            && near(bounds, expected.bounds)
            && screens.work_area(bounds) == Some(expected.area),
        "Window changed before the group update"
    );
    Ok((pid, window))
}

pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    let (_, window) = revalidate(expected)?;
    let size = CGSize::new(target.width as f64, target.height as f64);
    set_size(&window, &size)?;
    set_position(&window, &CGPoint::new(target.x as f64, target.y as f64))?;
    // Some applications constrain the first resize against the previous monitor.
    set_size(&window, &size)?;
    read_rect(&window)
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    let (pid, window) = revalidate(expected)?;
    activate_window(pid, &window)
}

pub fn move_tiling_window(expected: &super::TilingWindow, target: Rect) -> Result<Rect> {
    move_snapshot(&expected.snapshot, target)
}

/// Space key for a set of space IDs: one Space, or a window shown on all of them.
fn space_key(ids: &[i64]) -> Option<String> {
    match ids {
        [] => None,
        [id] => Some(format!("macos:{id}")),
        _ => Some("macos:all".to_owned()),
    }
}

const CURRENT_SPACE: &str = "macos:current";

/// Space keys per window from the private SkyLight Spaces API, looked up at
/// run time: if macOS drops it, every window shares one layout per monitor.
fn window_spaces(numbers: &[u32]) -> std::collections::HashMap<u32, String> {
    type Connection = unsafe extern "C" fn() -> i32;
    type CopySpaces = unsafe extern "C" fn(i32, i32, CFTypeRef) -> CFTypeRef;
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    const ALL_SPACES: i32 = 7;
    let lookup = |names: [&std::ffi::CStr; 2]| {
        names.into_iter().find_map(|name| {
            // SAFETY: dlsym only reads the NUL-terminated name.
            let symbol = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
            (!symbol.is_null()).then_some(symbol)
        })
    };
    let mut spaces = std::collections::HashMap::new();
    let (Some(connection), Some(copy)) = (
        lookup([c"SLSMainConnectionID", c"CGSMainConnectionID"]),
        lookup([c"SLSCopySpacesForWindows", c"CGSCopySpacesForWindows"]),
    ) else {
        return spaces;
    };
    // SAFETY: Both symbols have these C signatures in every macOS release that
    // exports them. The window array outlives each call, and the result is an
    // owned (+1) CFArray of CFNumbers or null.
    unsafe {
        let connection: Connection = std::mem::transmute(connection);
        let copy: CopySpaces = std::mem::transmute(copy);
        let cid = connection();
        for &number in numbers {
            let windows = CFArray::from_CFTypes(&[CFNumber::from(i64::from(number))]);
            let raw = copy(cid, ALL_SPACES, windows.as_CFTypeRef());
            if raw.is_null() {
                continue;
            }
            let ids: CFArray<CFNumber> = CFArray::wrap_under_create_rule(raw.cast());
            let ids: Vec<i64> = ids.iter().filter_map(|id| id.to_i64()).collect();
            if let Some(key) = space_key(&ids) {
                spaces.insert(number, key);
            }
        }
    }
    spaces
}

/// Standard, restored, resizable windows of other applications on the visible
/// Spaces. Identities also cover these applications' minimized windows.
pub fn tiling_desktop() -> Result<super::TilingDesktop> {
    trusted()?;
    let screens = ScreenInfo::read(main_thread()?)?;
    let own = std::process::id() as i32;
    let listed: Vec<_> = listed_windows()?
        .into_iter()
        .filter(|w| w.layer == 0 && w.pid != own)
        .collect();
    let spaces = window_spaces(&listed.iter().map(|w| w.number).collect::<Vec<_>>());
    let space = |number: u32| {
        spaces
            .get(&number)
            .cloned()
            .unwrap_or_else(|| CURRENT_SPACE.to_owned())
    };
    let mut result = super::TilingDesktop {
        // A held left button means a drag or resize may be in progress.
        // SAFETY: Reads the combined session's left mouse button; no pointers.
        interacting: unsafe { CGEventSourceButtonState(0, 0) },
        ..Default::default()
    };
    for window in &listed {
        let key = space(window.number);
        if !result.workspaces.contains(&key) {
            result.workspaces.push(key);
        }
    }
    let mut pids: Vec<i32> = listed.iter().map(|w| w.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    for pid in pids {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|app| app.localizedName())
            .map(|name| name.to_string())
            .unwrap_or_default();
        for window in standard_windows(pid) {
            let Some(number) = window_number(&window) else {
                continue;
            };
            result.identities.push(window_id(pid, number));
            if flag(&window, "AXMinimized")
                || flag(&window, "AXFullScreen")
                || !settable(&window, "AXSize")
                || !listed.iter().any(|w| w.number == number)
            {
                continue;
            }
            let Ok(bounds) = read_rect(&window) else {
                continue;
            };
            let Some(area) = screens.work_area(bounds) else {
                continue;
            };
            result.windows.push(super::TilingWindow {
                snapshot: WindowSnapshot {
                    handle: u64::from(number),
                    id: window_id(pid, number),
                    area,
                    bounds,
                },
                workspace: space(number),
                app: app.clone(),
                size_step: (1, 1),
            });
        }
    }
    result.windows.sort_by_key(|w| w.snapshot.handle);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_keys_name_one_space_or_all() {
        assert_eq!(space_key(&[]), None);
        assert_eq!(space_key(&[42]).as_deref(), Some("macos:42"));
        assert_eq!(space_key(&[1, 2]).as_deref(), Some("macos:all"));
        assert_eq!(window_id(12, 345), "ax:12:345");
    }

    #[test]
    fn typed_values_round_trip_and_reject_mismatches() {
        let point = CGPoint::new(-120.5, 480.25);
        let size = CGSize::new(800.0, 600.0);
        let position = position_value(&point).unwrap();
        let dimensions = size_value(&size).unwrap();
        let decoded_point = read_position(&position).unwrap();
        let decoded_size = read_size(&dimensions).unwrap();
        assert_eq!((decoded_point.x, decoded_point.y), (point.x, point.y));
        assert_eq!(
            (decoded_size.width, decoded_size.height),
            (size.width, size.height)
        );
        assert!(read_position(&dimensions).is_err());
        assert!(read_size(&position).is_err());
        let string = CFString::new("not an AXValue").as_CFType();
        assert!(read_position(&string).is_err());
        assert!(read_size(&string).is_err());
        assert!(attribute(&string, "AXPosition").is_err());
        assert!(set_position(&string, &point).is_err());
    }
}
