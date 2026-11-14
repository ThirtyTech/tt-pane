//! Conservative enumeration for automatic tiling; never restores windows.
use super::*;
use crate::platform::{TilingDesktop, TilingWindow};
use windows::{
    Win32::{
        Foundation::{CloseHandle, LPARAM, RPC_E_CHANGED_MODE},
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
        UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager},
    },
    core::{BOOL, PWSTR},
};

struct Com(bool);
impl Com {
    fn enter() -> Result<Self> {
        // SAFETY: Initializes only the calling thread; successful calls are balanced.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self(false));
        }
        result.ok()?;
        Ok(Self(true))
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: Balances this guard's successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn manager() -> Result<IVirtualDesktopManager> {
    // SAFETY: COM is initialized by callers; no aggregate or retained Rust pointers.
    Ok(unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL) }?)
}

pub fn tiling_desktop() -> Result<TilingDesktop> {
    let _com = Com::enter()?;
    let manager = manager()?;
    let _dpi = DpiContext::enter();
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: EnumWindows synchronously borrows this live exclusive Vec.
        unsafe { &mut *(data.0 as *mut Vec<HWND>) }.push(window);
        BOOL(1)
    }
    let mut result = TilingDesktop::default();
    // SAFETY: All Win32 calls validate opaque handles. Output buffers are live,
    // initialized and correctly sized. No borrowed pointers outlive these calls.
    unsafe {
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(0, &mut gui).is_ok() {
            result.interacting = gui.flags & GUI_INMOVESIZE != GUITHREADINFO_FLAGS(0);
        }
        let mut handles = Vec::<HWND>::new();
        EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize))?;
        for window in handles {
            if !manager
                .IsWindowOnCurrentVirtualDesktop(window)
                .is_ok_and(|v| v.as_bool())
            {
                continue;
            }
            let Ok(desktop) = manager.GetWindowDesktopId(window) else {
                continue;
            };
            if desktop == windows::core::GUID::zeroed() {
                continue;
            }
            let workspace = format!("{desktop:?}");
            if !result.workspaces.contains(&workspace) {
                result.workspaces.push(workspace.clone());
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            result
                .identities
                .push(format!("win32:{pid}:{}", window.0 as usize));
            if pid == std::process::id()
                || GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid())
                || GetWindowLongPtrW(window, GWL_STYLE) as u32 & WS_THICKFRAME.0 == 0
            {
                continue;
            }
            let mut class = [0u16; 256];
            let count = GetClassNameW(window, &mut class);
            if String::from_utf16_lossy(&class[..count.max(0) as usize]) == "#32770" {
                continue;
            }
            let Ok(Some(snapshot)) = window_snapshot(window.0 as usize as u64) else {
                continue;
            };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(
                MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                &mut info,
            )
            .as_bool()
            {
                continue;
            }
            let monitor = from_rect(info.rcMonitor);
            let bounds = snapshot.bounds;
            if bounds.x <= monitor.x
                && bounds.y <= monitor.y
                && i64::from(bounds.x) + i64::from(bounds.width)
                    >= i64::from(monitor.x) + i64::from(monitor.width)
                && i64::from(bounds.y) + i64::from(bounds.height)
                    >= i64::from(monitor.y) + i64::from(monitor.height)
            {
                continue;
            }
            let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                continue;
            };
            let mut name = [0u16; 32768];
            let mut length = name.len() as u32;
            let queried = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(name.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(process);
            if queried.is_err() {
                continue;
            }
            let path = String::from_utf16_lossy(&name[..length as usize]);
            let app = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned();
            result.windows.push(TilingWindow {
                snapshot,
                workspace,
                app,
            });
        }
    }
    result.windows.sort_by_key(|w| w.snapshot.handle);
    Ok(result)
}

pub fn move_tiling_window(expected: &TilingWindow, target: Rect) -> Result<Rect> {
    let _com = Com::enter()?;
    let manager = manager()?;
    let window = HWND(usize::try_from(expected.snapshot.handle)? as *mut _);
    // SAFETY: COM validates the opaque window handle and returns owned values.
    unsafe {
        ensure!(
            manager.IsWindowOnCurrentVirtualDesktop(window)?.as_bool()
                && format!("{:?}", manager.GetWindowDesktopId(window)?) == expected.workspace,
            "Window left the current virtual desktop"
        );
    }
    move_snapshot(&expected.snapshot, target)
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires interactive Windows desktop; reads desktop eligibility and moves only a disposable fixture"]
    fn native_desktop_filter_and_guarded_move() {
        crate::test_support::with_focus_windows::<1>(|[handle]| {
            let desktop = tiling_desktop().unwrap();
            assert!(!desktop.windows.iter().any(|w| w.snapshot.handle == handle)); // own process excluded
            let _com = Com::enter().unwrap();
            let manager = manager().unwrap();
            let window = HWND(handle as usize as *mut _);
            // SAFETY: The fixture owns this live window; COM is initialized above.
            let workspace = unsafe { format!("{:?}", manager.GetWindowDesktopId(window).unwrap()) };
            let snapshot = window_snapshot(handle).unwrap().unwrap();
            let target = Rect {
                x: snapshot.bounds.x + 15,
                ..snapshot.bounds
            };
            let mut expected = TilingWindow {
                snapshot,
                workspace,
                app: "fixture.exe".into(),
            };
            assert_eq!(move_tiling_window(&expected, target).unwrap(), target);
            expected.snapshot = window_snapshot(handle).unwrap().unwrap();
            expected.workspace = "wrong-desktop".into();
            assert!(
                move_tiling_window(
                    &expected,
                    Rect {
                        x: target.x + 15,
                        ..target
                    }
                )
                .is_err()
            );
            assert_eq!(window_snapshot(handle).unwrap().unwrap().bounds, target);
        });
    }
}
