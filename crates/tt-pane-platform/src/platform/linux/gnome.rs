//! GNOME on Wayland: the bundled Shell extension owns window control and is
//! reached over the session bus (see packaging/gnome).

use crate::{
    FocusVisibility,
    geometry::{Action, Rect},
    visibility,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use zbus::blocking::Connection;

const BUS_NAME: &str = "net.thirtytech.TtPane";
const OBJECT_PATH: &str = "/net/thirtytech/TtPane";
const INTERFACE: &str = "net.thirtytech.TtPane";

const MISSING: &str = "The tt-pane GNOME Shell extension is not running. Open tt-pane Settings (or run `tt-pane gnome setup`), then log out and back in so GNOME loads it.";
const OUTDATED: &str = "The running tt-pane GNOME Shell extension is out of date. Open tt-pane Settings (or run `tt-pane gnome setup`), then log out and back in so GNOME loads the new version.";

/// GNOME reports itself as e.g. "GNOME" or "ubuntu:GNOME".
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")))
}

/// A restored application window on the active workspace, identified by its
/// Mutter window ID (stable for the window's lifetime).
#[derive(Debug, Clone)]
pub struct WindowSnapshot {
    pub handle: u64,
    pub id: String,
    pub area: Rect,
    pub bounds: Rect,
}

type Wire = (i32, i32, i32, i32);
/// ListWindows entry: id, frame, work area, monitor index, application window.
type WireWindow = (u64, Wire, Wire, i32, bool);

fn rect((x, y, width, height): Wire) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn window_id(handle: u64) -> String {
    format!("gnome:{handle}")
}

// The tray polls centered groups ten times a second; reuse one bus connection.
fn connection() -> Result<Connection> {
    static SHARED: Mutex<Option<Connection>> = Mutex::new(None);
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(connection) = shared.as_ref() {
        return Ok(connection.clone());
    }
    let connection = Connection::session().context("Connect to the D-Bus session bus")?;
    *shared = Some(connection.clone());
    Ok(connection)
}

fn call<B, R>(connection: &Connection, method: &str, body: &B) -> Result<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let reply = connection
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), method, body)
        .map_err(|error| match &error {
            zbus::Error::MethodError(name, _, _)
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.ServiceUnknown"
                        | "org.freedesktop.DBus.Error.NameHasNoOwner"
                        | "org.freedesktop.DBus.Error.UnknownObject"
                ) =>
            {
                anyhow::anyhow!(MISSING)
            }
            // An extension loaded before this tt-pane lacks the newer methods.
            zbus::Error::MethodError(name, _, _)
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                anyhow::anyhow!(OUTDATED)
            }
            // The extension's thrown errors arrive as the method error's message.
            zbus::Error::MethodError(_, Some(message), _) => anyhow::anyhow!("{message}"),
            _ => anyhow::Error::new(error).context("Call the tt-pane GNOME Shell extension"),
        })?;
    reply
        .body()
        .deserialize()
        .context("Unexpected reply from the tt-pane GNOME Shell extension")
}

fn near(a: Rect, b: Rect) -> bool {
    [
        a.x.abs_diff(b.x),
        a.y.abs_diff(b.y),
        a.width.abs_diff(b.width),
        a.height.abs_diff(b.height),
    ]
    .into_iter()
    .all(|difference| difference <= 2)
}

/// Frame and work area of an available window, or None when it is closed,
/// minimized, maximized, fullscreen or on another workspace.
fn read_window(connection: &Connection, handle: u64) -> Result<Option<(Rect, Rect)>> {
    let (available, frame, area): (bool, Wire, Wire) = call(connection, "GetWindow", &(handle,))?;
    Ok(available.then(|| (rect(frame), rect(area))))
}

/// Ask GNOME to move a window, then wait for the client to commit the new
/// frame: Wayland clients resize asynchronously, and callers compare the
/// result with later observations.
fn move_resize(connection: &Connection, handle: u64, before: Rect, target: Rect) -> Result<Rect> {
    call::<_, ()>(
        connection,
        "MoveResize",
        &(handle, target.x, target.y, target.width, target.height),
    )?;
    if near(before, target) {
        return Ok(target);
    }
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut last = None;
    let mut stable = 0;
    while Instant::now() < deadline {
        if let Some((frame, _)) = read_window(connection, handle)? {
            if near(frame, target) {
                return Ok(frame);
            }
            // An app that constrains its size settles away from both rectangles.
            if !near(frame, before) {
                stable = if last == Some(frame) { stable + 1 } else { 0 };
                if stable >= 3 {
                    return Ok(frame);
                }
            }
            last = Some(frame);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(last.filter(|frame| !near(*frame, before)).unwrap_or(target))
}

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    let connection = connection()?;
    let (window, frame, area, fullscreen): (u64, Wire, Wire, bool) =
        call(&connection, "GetFocusedWindow", &())?;
    if fullscreen {
        bail!("Exit fullscreen before positioning this window");
    }
    let frame = rect(frame);
    let target = resolve(&window_id(window), rect(area), frame)?;
    // Pin the move to the window captured above, even if focus changed meanwhile.
    move_resize(&connection, window, frame, target)
}

pub fn foreground_handle() -> Option<u64> {
    let connection = connection().ok()?;
    call::<_, u64>(&connection, "GetFocusedId", &())
        .ok()
        .filter(|&id| id != 0)
}

/// Read without restoring, activating, or repositioning a window. Unavailable
/// windows are excluded so group updates cannot resurrect them.
pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    Ok(
        read_window(&connection()?, handle)?.map(|(bounds, area)| WindowSnapshot {
            handle,
            id: window_id(handle),
            area,
            bounds,
        }),
    )
}

fn revalidate(connection: &Connection, expected: &WindowSnapshot) -> Result<()> {
    let (bounds, area) =
        read_window(connection, expected.handle)?.context("Window is no longer available")?;
    ensure!(
        area == expected.area && bounds == expected.bounds,
        "Window changed before the group update"
    );
    Ok(())
}

/// Revalidate immediately before a group move, never overwriting a manual move
/// made since the coordinator's last observation.
pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    let connection = connection()?;
    revalidate(&connection, expected)?;
    move_resize(&connection, expected.handle, expected.bounds, target)
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    let connection = connection()?;
    revalidate(&connection, expected)?;
    activate(&connection, expected.handle)
}

/// Confirm activation before callers sample focus or arrange a group.
fn activate(connection: &Connection, handle: u64) -> Result<()> {
    call::<_, ()>(connection, "Activate", &(handle,))?;
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        if call::<_, u64>(connection, "GetFocusedId", &())? == handle {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Window activation did not complete within 500 ms"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Debug, Clone, Copy)]
struct Listed {
    handle: u64,
    frame: Rect,
    area: Rect,
    monitor: i32,
    application: bool,
}

/// Showing windows on the active workspace, topmost first, plus monitor rectangles.
fn list(connection: &Connection) -> Result<(Vec<Listed>, Vec<Rect>)> {
    let (windows, monitors): (Vec<WireWindow>, Vec<Wire>) = call(connection, "ListWindows", &())?;
    Ok((
        windows
            .into_iter()
            .map(|(handle, frame, area, monitor, application)| Listed {
                handle,
                frame: rect(frame),
                area: rect(area),
                monitor,
                application,
            })
            .collect(),
        monitors.into_iter().map(rect).collect(),
    ))
}

/// Mirrors the Windows candidate rules over GNOME's stacking order.
fn candidates(
    selected: u64,
    windows: &[Listed],
    screens: &[Rect],
    visibility: FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<(Rect, Vec<u64>, Vec<Rect>)> {
    let selected = windows
        .iter()
        .find(|w| w.handle == selected)
        .context("Select a window on the active workspace to change focus")?;
    let current = selected.frame;
    let hold = visibility == FocusVisibility::Hold;
    let snapped_only =
        hold && (!group.is_empty() || !visibility::is_snapped(current, Some(selected.area)));
    let middle_only = hold && !snapped_only;
    let visibility = if hold {
        FocusVisibility::IncludeHidden
    } else {
        visibility
    };
    let mut above = Vec::new();
    let mut handles = Vec::new();
    let mut bounds = Vec::new();
    for window in windows {
        let is_middle = middle_members.contains(&window.handle);
        let selectable = window.handle != selected.handle
            && window.application
            && ((!snapped_only && group.is_empty())
                || (!group.contains(&window.handle) && window.monitor == selected.monitor));
        let work_area = (selectable
            && (snapped_only || middle_only || visibility != FocusVisibility::IncludeHidden))
            .then_some(window.area);
        let snapped = visibility::is_snapped(window.frame, work_area);
        if selectable
            && (!snapped_only || (!is_middle && snapped))
            && (!middle_only || is_middle || !snapped)
            && (visibility != FocusVisibility::BackgroundOnly || !is_middle)
            && visibility::focus_eligible(window.frame, work_area, screens, &above, visibility)
        {
            handles.push(window.handle);
            bounds.push(window.frame);
        }
        // Panels, docks and the selected window still cover windows beneath them.
        if visibility != FocusVisibility::IncludeHidden {
            above.push(window.frame);
        }
    }
    Ok((current, handles, bounds))
}

fn selected_handle(connection: &Connection, handle: Option<u64>) -> Result<u64> {
    match handle {
        Some(handle) => Ok(handle),
        None => match call::<_, u64>(connection, "GetFocusedId", &())? {
            0 => bail!("No focused application window"),
            id => Ok(id),
        },
    }
}

pub fn focus_eligible_handles(handle: u64, visibility: FocusVisibility) -> Result<Vec<u64>> {
    let connection = connection()?;
    let (windows, screens) = list(&connection)?;
    Ok(candidates(handle, &windows, &screens, visibility, &[], &[])?.1)
}

pub fn focus_window(
    handle: Option<u64>,
    action: Action,
    visibility: FocusVisibility,
    group: &[u64],
    middle_members: &[u64],
) -> Result<()> {
    let connection = connection()?;
    let selected = selected_handle(&connection, handle)?;
    let (windows, screens) = list(&connection)?;
    let (current, handles, bounds) = candidates(
        selected,
        &windows,
        &screens,
        visibility,
        group,
        middle_members,
    )?;
    let neighbor = if group.is_empty() {
        crate::geometry::focus_neighbor(current, &bounds, action)
    } else {
        crate::geometry::focus_group_exit(current, &bounds, action)
    };
    if let Some(index) = neighbor {
        activate(&connection, handles[index])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 32,
        width: 3840,
        height: 2128,
    };

    fn window(handle: u64, frame: Rect) -> Listed {
        Listed {
            handle,
            frame,
            area: AREA,
            monitor: 0,
            application: true,
        }
    }

    #[test]
    fn candidates_follow_stacking_and_skip_non_applications() {
        let left = Action::Left.target(AREA, AREA);
        let right = Action::Right.target(AREA, AREA);
        let middle = Action::Center.target(AREA, AREA);
        let panel = Listed {
            application: false,
            ..window(
                9,
                Rect {
                    x: 0,
                    y: 0,
                    width: 3840,
                    height: 32,
                },
            )
        };
        // Topmost first: the middle window covers part of both halves.
        let windows = [panel, window(1, middle), window(2, left), window(3, right)];
        let (current, handles, bounds) = candidates(
            2,
            &windows,
            &[Rect {
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
            }],
            FocusVisibility::BackgroundOnly,
            &[],
            &[1],
        )
        .unwrap();
        assert_eq!(current, left);
        assert_eq!(handles, [3]);
        assert_eq!(
            crate::geometry::focus_neighbor(current, &bounds, Action::FocusRight),
            Some(0)
        );
        assert!(candidates(7, &windows, &[], FocusVisibility::IncludeHidden, &[], &[]).is_err());
    }

    #[test]
    fn group_exits_stay_on_the_selected_monitor() {
        let other = Listed {
            monitor: 1,
            ..window(4, Rect { x: 3840, ..AREA })
        };
        let windows = [
            window(1, Action::Center.target(AREA, AREA)),
            window(2, AREA),
            other,
        ];
        let (_, handles, _) =
            candidates(1, &windows, &[], FocusVisibility::IncludeHidden, &[1], &[1]).unwrap();
        assert_eq!(handles, [2]);
    }
}
