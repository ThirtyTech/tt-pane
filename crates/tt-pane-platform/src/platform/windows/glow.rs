//! Short-lived, per-pixel-alpha cue. All native resources stay on the owning thread.
use super::{DpiContext, read_bounds};
use anyhow::{Result, ensure};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{Controls::Dialogs::*, WindowsAndMessaging::*},
    },
    core::w,
};

static GENERATION: AtomicU64 = AtomicU64::new(0);
static WORKER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub(super) fn is_glow(window: HWND) -> bool {
    // SAFETY: The property is an opaque marker, never dereferenced.
    unsafe { !GetPropW(window, w!("tt-pane.focus-glow")).is_invalid() }
}

pub fn show_focus_glow(handle: u64, color: u32) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut worker) = WORKER.lock() {
        *worker = thread::Builder::new()
            .name("focus-glow".into())
            .spawn(move || {
                // A visual cue must never turn a successful window action into a failure.
                let _ = animate(handle, color, generation);
            })
            .ok();
    }
}

/// Keep standalone CLI processes alive until their cue has finished.
pub fn wait_for_focus_glow() {
    let worker = WORKER.lock().ok().and_then(|mut worker| worker.take());
    if let Some(worker) = worker {
        let _ = worker.join();
    }
}

pub fn choose_glow_color(rgb: u32) -> Option<u32> {
    let mut custom = [COLORREF(0); 16];
    let mut dialog = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        rgbResult: COLORREF(rgb.swap_bytes() >> 8),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_FULLOPEN | CC_RGBINIT,
        ..Default::default()
    };
    // SAFETY: The modal dialog retains neither the initialized structure nor its buffer.
    unsafe {
        dialog.hwndOwner = GetForegroundWindow();
        ChooseColorW(&mut dialog)
            .as_bool()
            .then(|| dialog.rgbResult.0.swap_bytes() >> 8)
    }
}

fn envelope(elapsed: f32) -> u8 {
    let strength = if elapsed < 0.08 {
        elapsed / 0.08
    } else {
        (0.48 - elapsed) / 0.40
    };
    let t = strength.clamp(0.0, 1.0);
    (255.0 * t * t * (3.0 - 2.0 * t)).round() as u8
}

/// Premultiplied BGRA. A small inner fringe keeps cues visible at monitor edges.
fn pixel(x: i32, y: i32, width: i32, height: i32, pad: i32, color: u32) -> u32 {
    let dx = (pad - x).max(x - (width - pad - 1)).max(0) as f32;
    let dy = (pad - y).max(y - (height - pad - 1)).max(0) as f32;
    let distance = if dx > 0.0 || dy > 0.0 {
        (dx * dx + dy * dy).sqrt()
    } else {
        -((x - pad)
            .min(width - pad - 1 - x)
            .min(y - pad)
            .min(height - pad - 1 - y) as f32)
    };
    let spread = if distance < 0.0 { 3.0 } else { pad as f32 };
    let t = (1.0 - distance.abs() / spread).clamp(0.0, 1.0);
    let alpha = (180.0 * t * t) as u32;
    let r = ((color >> 16) & 255) * alpha / 255;
    let g = ((color >> 8) & 255) * alpha / 255;
    let b = (color & 255) * alpha / 255;
    (alpha << 24) | (r << 16) | (g << 8) | b
}

#[derive(Default)]
struct Surface {
    window: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: Owned handles are used and released only on their creating thread.
        unsafe {
            if !self.window.is_invalid() {
                let _ = DestroyWindow(self.window);
            }
            if !self.previous.is_invalid() {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
        }
    }
}

fn animate(handle: u64, color: u32, generation: u64) -> Result<()> {
    let target = HWND(usize::try_from(handle)? as *mut _);
    let _dpi = DpiContext::enter();
    // SAFETY: Win32 validates HWNDs. Buffers are sized from checked dimensions;
    // the DIB pointer remains valid until Surface drops, after all writes finish.
    unsafe {
        if !IsWindow(Some(target)).as_bool() || IsIconic(target).as_bool() {
            return Ok(());
        }
        let foreground = GetForegroundWindow();
        let (_, bounds) = read_bounds(target)?;
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(target).max(96);
        let pad = (14 * dpi / 96) as i32;
        let width = bounds.right - bounds.left + pad * 2;
        let height = bounds.bottom - bounds.top + pad * 2;
        ensure!(
            width > 0 && height > 0 && i64::from(width) * i64::from(height) <= 40_000_000,
            "Invalid glow bounds"
        );
        let mut surface = Surface::default();
        surface.window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("tt-pane focus glow"),
            WS_POPUP,
            bounds.left - pad,
            bounds.top - pad,
            width,
            height,
            None,
            None,
            None,
            None,
        )?;
        SetPropW(
            surface.window,
            w!("tt-pane.focus-glow"),
            Some(HANDLE(1usize as *mut _)),
        )?;
        surface.dc = CreateCompatibleDC(None);
        ensure!(!surface.dc.is_invalid(), "Create glow DC");
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        surface.bitmap =
            CreateDIBSection(Some(surface.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        ensure!(!bits.is_null(), "Create glow bitmap");
        surface.previous = SelectObject(surface.dc, surface.bitmap.into());
        let pixels =
            std::slice::from_raw_parts_mut(bits.cast::<u32>(), width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                pixels[(y * width + x) as usize] = pixel(x, y, width, height, pad, color);
            }
        }
        let destination = POINT {
            x: bounds.left - pad,
            y: bounds.top - pad,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let origin = POINT::default();
        let start = Instant::now();
        let mut shown = false;
        while start.elapsed() < Duration::from_millis(480)
            && GENERATION.load(Ordering::SeqCst) == generation
        {
            if GetForegroundWindow() != foreground
                || !IsWindowVisible(target).as_bool()
                || IsIconic(target).as_bool()
                || !read_bounds(target).is_ok_and(|(_, current)| current == bounds)
            {
                break;
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                SourceConstantAlpha: envelope(start.elapsed().as_secs_f32()),
                AlphaFormat: AC_SRC_ALPHA as u8,
                BlendFlags: 0,
            };
            UpdateLayeredWindow(
                surface.window,
                None,
                Some(&destination),
                Some(&size),
                Some(surface.dc),
                Some(&origin),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
            if !shown {
                SetWindowPos(
                    surface.window,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
                shown = true;
            }
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(16));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires interactive desktop; temporarily focuses disposable windows"]
    fn cross_thread_activation_completes_before_glow_starts() {
        use std::sync::mpsc;
        // Own windows on separate input queues, like separate tiled applications.
        // SAFETY: No pointers are passed; the returned HWND is borrowed.
        let original = unsafe { GetForegroundWindow() };
        let (send, recv) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        // SAFETY: This thread owns and pumps the disposable native window.
        let owner = thread::spawn(move || unsafe {
            let window = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Focus timing test"),
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
            let _owned = Surface {
                window,
                ..Default::default()
            };
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            send.send(window.0 as usize).unwrap();
            while matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                // Model an app busy painting its newly activated window.
                thread::sleep(Duration::from_millis(100));
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        });
        let target = HWND(recv.recv().unwrap() as *mut _);
        let activation = super::super::activate_window(target);
        // SAFETY: Read the borrowed foreground HWND after the activation helper returns.
        let immediate = unsafe { GetForegroundWindow() };
        let started = Instant::now();
        let glow = animate(
            target.0 as usize as u64,
            0xf2f0e8,
            GENERATION.load(Ordering::SeqCst),
        );
        let duration = started.elapsed();
        // SAFETY: Restore the original foreground window before checking results.
        unsafe {
            let _ = SetForegroundWindow(original);
        }
        stop.send(()).unwrap();
        owner.join().unwrap();
        activation.unwrap();
        assert_eq!(
            immediate, target,
            "Activation returned before its input queue processed the request"
        );
        glow.unwrap();
        assert!(
            duration >= Duration::from_millis(450),
            "Glow canceled before completing: {duration:?}"
        );
    }
    #[test]
    #[ignore = "requires an interactive Windows desktop; shows a disposable window and glow"]
    fn native_glow_preserves_focus_and_cleans_up() {
        // SAFETY: The test owns its disposable window and releases it on this thread.
        unsafe {
            let target = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Glow test target"),
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
            let owned = Surface {
                window: target,
                ..Default::default()
            };
            let _ = ShowWindow(target, SW_SHOWNOACTIVATE);
            let foreground = GetForegroundWindow();
            let observer = thread::spawn(|| {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    if let Ok(overlay) = FindWindowW(None, w!("tt-pane focus glow")) {
                        if IsWindowVisible(overlay).as_bool() {
                            let style = GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32;
                            for flag in [
                                WS_EX_TRANSPARENT,
                                WS_EX_NOACTIVATE,
                                WS_EX_LAYERED,
                                WS_EX_TOOLWINDOW,
                            ] {
                                assert_ne!(style & flag.0, 0);
                            }
                            assert!(is_glow(overlay));
                            return;
                        }
                    }
                    assert!(Instant::now() < deadline, "Glow was never visible");
                    thread::sleep(Duration::from_millis(5));
                }
            });
            let started = Instant::now();
            animate(
                target.0 as usize as u64,
                0xf2f0e8,
                GENERATION.load(Ordering::SeqCst),
            )
            .unwrap();
            observer.join().unwrap();
            assert!(started.elapsed() >= Duration::from_millis(450));
            assert_eq!(GetForegroundWindow(), foreground);
            assert!(FindWindowW(None, w!("tt-pane focus glow")).is_err());
            drop(owned);
        }
    }
    #[test]
    fn glow_is_transparent_inside_and_fades_outward() {
        assert_eq!(pixel(50, 50, 100, 100, 14, 0xffffff), 0);
        let edge = pixel(14, 50, 100, 100, 14, 0xff8040);
        let outer = pixel(7, 50, 100, 100, 14, 0xff8040);
        assert!(edge >> 24 > outer >> 24);
        assert_eq!(pixel(0, 50, 100, 100, 14, 0xffffff), 0);
        for shift in [0, 8, 16] {
            assert!(((edge >> shift) & 255) <= edge >> 24);
        }
    }
    #[test]
    fn cue_fades_in_then_out() {
        assert_eq!(envelope(0.0), 0);
        assert_eq!(envelope(0.08), 255);
        assert!(envelope(0.24) < 255);
        assert_eq!(envelope(0.48), 0);
    }
}
