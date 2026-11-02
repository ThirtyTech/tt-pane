use crate::geometry::{Action, Rect, monitor_for};
use anyhow::{Context, Result, ensure};
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};
use core_graphics::geometry::{CGPoint, CGSize};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, name: CFStringRef, value: CFTypeRef)
    -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut c_void) -> bool;
}

fn attribute(element: &CFType, key: &str) -> Result<CFType> {
    let mut value = std::ptr::null();
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
    Ok(unsafe { CFType::wrap_under_create_rule(value) })
}

fn set_value<T>(window: &CFType, key: &str, kind: u32, value: &T) -> Result<()> {
    let raw = unsafe { AXValueCreate(kind, value as *const T as *const c_void) };
    ensure!(!raw.is_null(), "Could not create Accessibility value");
    let value = unsafe { CFType::wrap_under_create_rule(raw) };
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

pub fn apply(action: Action) -> Result<()> {
    ensure!(
        unsafe { AXIsProcessTrusted() },
        "Grant sTile Accessibility access in System Settings → Privacy & Security → Accessibility, then restart sTile"
    );
    let mtm =
        MainThreadMarker::new().context("Window control must run on the macOS main thread")?;
    let application = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .context("No frontmost application")?;
    let raw = unsafe { AXUIElementCreateApplication(application.processIdentifier()) };
    ensure!(!raw.is_null(), "Could not access frontmost application");
    let app = unsafe { CFType::wrap_under_create_rule(raw) };
    let window = attribute(&app, "AXFocusedWindow")?;
    let position = attribute(&window, "AXPosition")?;
    let size = attribute(&window, "AXSize")?;
    let mut point = CGPoint::new(0.0, 0.0);
    let mut dimensions = CGSize::new(0.0, 0.0);
    ensure!(
        unsafe {
            AXValueGetValue(
                position.as_CFTypeRef(),
                1,
                &mut point as *mut _ as *mut c_void,
            )
        },
        "Invalid AX position"
    );
    ensure!(
        unsafe {
            AXValueGetValue(
                size.as_CFTypeRef(),
                2,
                &mut dimensions as *mut _ as *mut c_void,
            )
        },
        "Invalid AX size"
    );
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
    let target = action.target(area, current);
    let size = CGSize::new(target.width as f64, target.height as f64);
    let point = CGPoint::new(target.x as f64, target.y as f64);
    set_value(&window, "AXSize", 2, &size)?;
    set_value(&window, "AXPosition", 1, &point)?;
    // Some applications constrain the first resize against the previous monitor.
    set_value(&window, "AXSize", 2, &size)?;
    Ok(())
}
