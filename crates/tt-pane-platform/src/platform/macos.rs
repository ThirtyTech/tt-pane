use crate::geometry::{Rect, monitor_for};
use anyhow::{Context, Result, ensure};
use core_foundation::{
    base::{CFType, CFTypeID, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};
use core_graphics::geometry::{CGPoint, CGSize};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
// SAFETY: These declarations match ApplicationServices AXUIElement.h/AXValue.h.
// Core Foundation Boolean is an unsigned byte; AXValueType is a 32-bit enum.
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
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

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    // SAFETY: This process-level query takes no arguments or pointers.
    let trusted = unsafe { AXIsProcessTrusted() } != 0;
    ensure!(
        trusted,
        "Grant tt-pane Accessibility access in System Settings → Privacy & Security → Accessibility, then restart tt-pane"
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

#[cfg(test)]
mod tests {
    use super::*;

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
