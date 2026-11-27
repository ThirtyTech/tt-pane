//! X11 sessions through EWMH: positioning the active window, and the window
//! inspection, moves and activation automatic tiling needs.

use super::WindowSnapshot;
use crate::{
    dock::DockEdge,
    geometry::{Rect, monitor_for},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    properties::{WmClass, WmSizeHints},
    protocol::{randr::ConnectionExt as _, xproto::*},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// `_NET_WM_DESKTOP` for windows shown on every desktop.
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

/// Marks a docked window, which tiling leaves alone in every tt-pane process.
const DOCKED: &str = "_TT_PANE_DOCKED";
/// On the root window: the strip the dock reserves, as `_NET_WM_STRUT_PARTIAL`.
const DOCK_STRUT: &str = "_TT_PANE_DOCK_STRUT";

struct X11 {
    connection: RustConnection,
    screen: usize,
    root: u32,
    display: String,
    atoms: Mutex<HashMap<&'static str, u32>>,
}

// The tray polls tiling ten times a second; reuse one connection.
static SHARED: Mutex<Option<Arc<X11>>> = Mutex::new(None);

fn shared() -> Result<Arc<X11>> {
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = shared.as_ref() {
        return Ok(x.clone());
    }
    let (connection, screen) = x11rb::connect(None).context("Connect to X11")?;
    let root = connection.setup().roots[screen].root;
    let x = Arc::new(X11 {
        connection,
        screen,
        root,
        display: std::env::var("DISPLAY").unwrap_or_default(),
        atoms: Mutex::new(HashMap::new()),
    });
    *shared = Some(x.clone());
    Ok(x)
}

/// Run against the shared connection, dropping it after a failure so a
/// restarted X server or window manager gets a fresh one.
fn with<T>(f: impl FnOnce(&X11) -> Result<T>) -> Result<T> {
    let x = shared()?;
    let result = f(&x);
    if result.is_err() {
        *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    result
}

/// What tiling needs to know about one client window.
struct Client {
    snapshot: WindowSnapshot,
    extents: [i32; 4],
    desktop: u32,
    pid: Option<u32>,
    app: String,
    /// WM_NORMAL_HINTS resize increments; (1, 1) when sizes are exact.
    size_step: (i32, i32),
    /// A viewable, restored, resizable top-level application window.
    tileable: bool,
}

/// `_NET_WM_STRUT_PARTIAL` of one client.
type Strut = [u32; 12];

impl X11 {
    fn atom(&self, name: &'static str) -> Result<u32> {
        let mut atoms = self.atoms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(atom) = atoms.get(name) {
            return Ok(*atom);
        }
        let atom = self
            .connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom;
        atoms.insert(name, atom);
        Ok(atom)
    }

    fn property(&self, window: u32, name: &'static str) -> Result<Vec<u32>> {
        let reply = self
            .connection
            .get_property(false, window, self.atom(name)?, AtomEnum::ANY, 0, 4096)?
            .reply()?;
        Ok(reply.value32().map(|v| v.collect()).unwrap_or_default())
    }

    fn message(&self, window: u32, name: &'static str, data: [u32; 5]) -> Result<()> {
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ClientMessageEvent::new(32, window, self.atom(name)?, data),
            )?
            .check()?;
        Ok(())
    }

    fn id(&self, window: u32) -> String {
        format!("x11:{}:{}:{window}", self.display, self.root)
    }

    fn require_ewmh(&self) -> Result<()> {
        let supported = self.property(self.root, "_NET_SUPPORTED")?;
        ensure!(
            supported.contains(&self.atom("_NET_MOVERESIZE_WINDOW")?),
            "Window manager does not support EWMH window positioning"
        );
        Ok(())
    }

    fn active(&self) -> Result<Option<u32>> {
        Ok(self
            .property(self.root, "_NET_ACTIVE_WINDOW")?
            .first()
            .copied()
            .filter(|&w| w != 0 && w != self.root))
    }

    /// Outer frame and the window manager's frame extents (left, right, top, bottom).
    fn frame(&self, window: u32) -> Result<(Rect, [i32; 4])> {
        let geometry = self.connection.get_geometry(window)?.reply()?;
        let translated = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)?
            .reply()?;
        let extents = self.property(window, "_NET_FRAME_EXTENTS")?;
        let edge = |i: usize| extents.get(i).copied().unwrap_or(0) as i32;
        let extents = [edge(0), edge(1), edge(2), edge(3)];
        Ok((
            Rect {
                x: i32::from(translated.dst_x) - extents[0],
                y: i32::from(translated.dst_y) - extents[2],
                width: i32::from(geometry.width) + extents[0] + extents[1],
                height: i32::from(geometry.height) + extents[2] + extents[3],
            },
            extents,
        ))
    }

    fn monitors(&self) -> Result<Vec<Rect>> {
        let monitors: Vec<Rect> = self
            .connection
            .randr_get_monitors(self.root, true)?
            .reply()?
            .monitors
            .into_iter()
            .map(|m| Rect {
                x: i32::from(m.x),
                y: i32::from(m.y),
                width: i32::from(m.width),
                height: i32::from(m.height),
            })
            .collect();
        ensure!(!monitors.is_empty(), "No RandR monitors found");
        Ok(monitors)
    }

    /// Client struts, plus the dock's reserved strip: GNOME Shell reserves it
    /// for its own chrome, which no client strut shows.
    fn struts(&self, clients: &[u32]) -> Vec<Strut> {
        let mut struts: Vec<Strut> = clients
            .iter()
            .chain([&self.root])
            .filter_map(|&client| {
                let name = if client == self.root {
                    DOCK_STRUT
                } else {
                    "_NET_WM_STRUT_PARTIAL"
                };
                let strut = self.property(client, name).ok()?;
                strut.get(..12)?.try_into().ok()
            })
            .collect();
        struts.extend(self.workarea_strut());
        struts
    }

    /// The window manager's `_NET_WORKAREA` for the current desktop, as a
    /// full-span strut. GNOME Shell's top bar is compositor chrome, not a
    /// client, so only this property accounts for it.
    fn workarea_strut(&self) -> Option<Strut> {
        let desktop = self
            .property(self.root, "_NET_CURRENT_DESKTOP")
            .ok()?
            .first()
            .copied()
            .unwrap_or(0) as usize;
        let areas = self.property(self.root, "_NET_WORKAREA").ok()?;
        let area: [u32; 4] = areas
            .get(desktop * 4..desktop * 4 + 4)
            .or_else(|| areas.get(..4))?
            .try_into()
            .ok()?;
        let screen = &self.connection.setup().roots[self.screen];
        workarea_strut(
            area,
            u32::from(screen.width_in_pixels),
            u32::from(screen.height_in_pixels),
        )
    }

    fn work_area(&self, monitor: Rect, struts: &[Strut]) -> Rect {
        let screen = &self.connection.setup().roots[self.screen];
        work_area(
            monitor,
            struts,
            i32::from(screen.width_in_pixels),
            i32::from(screen.height_in_pixels),
        )
    }

    fn area_for(&self, frame: Rect, monitors: &[Rect], struts: &[Strut]) -> Option<Rect> {
        monitor_for(frame, monitors).map(|index| self.work_area(monitors[index], struts))
    }

    fn inspect(&self, window: u32, monitors: &[Rect], struts: &[Strut]) -> Result<Client> {
        let attributes = self.connection.get_window_attributes(window)?.reply()?;
        let states = self.property(window, "_NET_WM_STATE")?;
        let has = |name| self.atom(name).is_ok_and(|atom| states.contains(&atom));
        let types = self.property(window, "_NET_WM_WINDOW_TYPE")?;
        let normal = types.first().is_none_or(|&t| {
            self.atom("_NET_WM_WINDOW_TYPE_NORMAL")
                .is_ok_and(|n| n == t)
        });
        let transient = self
            .connection
            .get_property(
                false,
                window,
                AtomEnum::WM_TRANSIENT_FOR,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()?
            .value32()
            .and_then(|mut v| v.next())
            .is_some_and(|owner| owner != 0);
        let hints = WmSizeHints::get_normal_hints(&self.connection, window)?
            .reply()
            .ok()
            .flatten();
        let fixed = hints
            .as_ref()
            .is_some_and(|h| h.min_size.is_some_and(|min| h.max_size == Some(min)));
        let (bounds, extents) = self.frame(window)?;
        let area = self.area_for(bounds, monitors, struts);
        let app = WmClass::get(&self.connection, window)?
            .reply()
            .ok()
            .flatten()
            .map(|class| String::from_utf8_lossy(class.class()).into_owned())
            .unwrap_or_default();
        let docked = !self.property(window, DOCKED)?.is_empty();
        let tileable = attributes.map_state == MapState::VIEWABLE
            && !docked
            && normal
            && !transient
            && !fixed
            && area.is_some()
            && !has("_NET_WM_STATE_HIDDEN")
            && !has("_NET_WM_STATE_FULLSCREEN")
            && !has("_NET_WM_STATE_SKIP_TASKBAR")
            && !(has("_NET_WM_STATE_MAXIMIZED_VERT") && has("_NET_WM_STATE_MAXIMIZED_HORZ"));
        Ok(Client {
            snapshot: WindowSnapshot {
                handle: u64::from(window),
                id: self.id(window),
                area: area.unwrap_or(bounds),
                bounds,
            },
            extents,
            desktop: self
                .property(window, "_NET_WM_DESKTOP")?
                .first()
                .copied()
                .unwrap_or(ALL_DESKTOPS),
            pid: self.property(window, "_NET_WM_PID")?.first().copied(),
            app,
            size_step: hints
                .and_then(|h| h.size_increment)
                .map_or((1, 1), |(w, h)| (w.max(1), h.max(1))),
            tileable,
        })
    }

    /// The window when it is still a restored, tileable client.
    fn live(&self, handle: u64) -> Result<Option<Client>> {
        let Ok(window) = u32::try_from(handle) else {
            return Ok(None);
        };
        let clients = self.property(self.root, "_NET_CLIENT_LIST")?;
        if !clients.contains(&window) {
            return Ok(None);
        }
        let client = self.inspect(window, &self.monitors()?, &self.struts(&clients))?;
        Ok(client.tileable.then_some(client))
    }

    fn revalidate(&self, expected: &WindowSnapshot) -> Result<Client> {
        let live = self
            .live(expected.handle)?
            .context("Window is no longer available")?;
        ensure!(
            live.snapshot.id == expected.id
                && live.snapshot.area == expected.area
                && live.snapshot.bounds == expected.bounds,
            "Window changed before the group update"
        );
        Ok(live)
    }

    /// Ask the window manager for an outer frame rectangle.
    fn move_frame(&self, window: u32, extents: [i32; 4], target: Rect) -> Result<()> {
        // NorthWest gravity, all four values, from a pager-like source.
        self.message(
            window,
            "_NET_MOVERESIZE_WINDOW",
            [
                1 | (0xf << 8) | (2 << 12),
                target.x as u32,
                target.y as u32,
                (target.width - extents[0] - extents[1]).max(1) as u32,
                (target.height - extents[2] - extents[3]).max(1) as u32,
            ],
        )?;
        self.connection.flush()?;
        Ok(())
    }
}

/// `[x, y, width, height]` of a work area as reserved space at each root
/// edge, spanning the whole root window.
fn workarea_strut([x, y, width, height]: [u32; 4], root_w: u32, root_h: u32) -> Option<Strut> {
    if width == 0 || height == 0 {
        return None;
    }
    let right = root_w.saturating_sub(x.saturating_add(width));
    let bottom = root_h.saturating_sub(y.saturating_add(height));
    let (last_x, last_y) = (root_w.saturating_sub(1), root_h.saturating_sub(1));
    Some([
        x, right, y, bottom, 0, last_y, 0, last_y, 0, last_x, 0, last_x,
    ])
}

/// Partial struts reserve only the monitors intersecting a panel's span.
fn work_area(monitor: Rect, struts: &[Strut], root_w: i32, root_h: i32) -> Rect {
    let mut area = monitor;
    let spans_y = |start: u32, end: u32| {
        (start as i32) < monitor.y + monitor.height && end as i32 >= monitor.y
    };
    let spans_x = |start: u32, end: u32| {
        (start as i32) < monitor.x + monitor.width && end as i32 >= monitor.x
    };
    for s in struts {
        let mut left = area.x;
        let mut top = area.y;
        let mut right = area.x + area.width;
        let mut bottom = area.y + area.height;
        if s[0] > 0 && spans_y(s[4], s[5]) {
            left = left.max(s[0] as i32);
        }
        if s[1] > 0 && spans_y(s[6], s[7]) {
            right = right.min(root_w - s[1] as i32);
        }
        if s[2] > 0 && spans_x(s[8], s[9]) {
            top = top.max(s[2] as i32);
        }
        if s[3] > 0 && spans_x(s[10], s[11]) {
            bottom = bottom.min(root_h - s[3] as i32);
        }
        if right > left && bottom > top {
            area = Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            };
        }
    }
    area
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

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    with(|x| {
        x.require_ewmh()?;
        let window = x.active()?.context("No active application window")?;
        let (current, extents) = x.frame(window)?;
        let monitors = x.monitors()?;
        let monitor =
            monitors[monitor_for(current, &monitors).context("No RandR monitors found")?];
        let area = x.work_area(monitor, &x.struts(&x.property(x.root, "_NET_CLIENT_LIST")?));
        let target = resolve(&x.id(window), area, current)?;
        x.message(
            window,
            "_NET_WM_STATE",
            [
                0,
                x.atom("_NET_WM_STATE_MAXIMIZED_VERT")?,
                x.atom("_NET_WM_STATE_MAXIMIZED_HORZ")?,
                2,
                0,
            ],
        )?;
        x.message(
            window,
            "_NET_WM_STATE",
            [0, x.atom("_NET_WM_STATE_FULLSCREEN")?, 0, 2, 0],
        )?;
        if let Some(duration) = crate::snap_animation() {
            crate::animation::run(
                current,
                target,
                duration,
                &mut |frame| x.move_frame(window, extents, frame),
                &mut crate::animation::pause,
            );
        }
        x.move_frame(window, extents, target)?;
        Ok(target)
    })
}

pub fn foreground_handle() -> Option<u64> {
    with(|x| x.active()).ok().flatten().map(u64::from)
}

pub fn window_snapshot(handle: u64) -> Result<Option<WindowSnapshot>> {
    with(|x| Ok(x.live(handle)?.map(|client| client.snapshot)))
}

/// Revalidate, move, then wait briefly for the window manager to apply the
/// new frame so later observations compare against what was committed.
pub fn move_snapshot(expected: &WindowSnapshot, target: Rect) -> Result<Rect> {
    with(|x| {
        let live = x.revalidate(expected)?;
        let window = expected.handle as u32;
        if let Some(duration) = crate::snap_animation() {
            crate::animation::run(
                expected.bounds,
                target,
                duration,
                &mut |frame| x.move_frame(window, live.extents, frame),
                &mut crate::animation::pause,
            );
        }
        x.move_frame(window, live.extents, target)?;
        if near(expected.bounds, target) {
            return Ok(target);
        }
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut last = None;
        let mut stable = 0;
        while Instant::now() < deadline {
            let (frame, _) = x.frame(window)?;
            if near(frame, target) {
                return Ok(frame);
            }
            // A window with size constraints settles away from both rectangles.
            if !near(frame, expected.bounds) {
                stable = if last == Some(frame) { stable + 1 } else { 0 };
                if stable >= 3 {
                    return Ok(frame);
                }
            }
            last = Some(frame);
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(last
            .filter(|frame| !near(*frame, expected.bounds))
            .unwrap_or(target))
    })
}

pub fn focus_snapshot(expected: &WindowSnapshot) -> Result<()> {
    with(|x| {
        x.revalidate(expected)?;
        let window = expected.handle as u32;
        // Source 2 (pager): window managers honor it without focus-stealing checks.
        x.message(window, "_NET_ACTIVE_WINDOW", [2, 0, 0, 0, 0])?;
        x.connection.flush()?;
        let deadline = Instant::now() + Duration::from_millis(500);
        while x.active()? != Some(window) {
            ensure!(
                Instant::now() < deadline,
                "Window activation did not complete within 500 ms"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })
}

/// Tileable clients on the current desktop. Identities cover every client on
/// it, minimized ones included, so floating choices survive a restore.
pub fn tiling_desktop() -> Result<crate::TilingDesktop> {
    with(|x| {
        x.require_ewmh()?;
        let current = x
            .property(x.root, "_NET_CURRENT_DESKTOP")?
            .first()
            .copied()
            .unwrap_or(0);
        let clients = x.property(x.root, "_NET_CLIENT_LIST")?;
        let monitors = x.monitors()?;
        let struts = x.struts(&clients);
        let pointer = x.connection.query_pointer(x.root)?.reply()?;
        let mut result = crate::TilingDesktop {
            interacting: u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) != 0,
            workspaces: vec![format!("x11:{current}"), "x11:all".to_owned()],
            ..Default::default()
        };
        let own = std::process::id();
        for window in clients {
            // A window can close mid-enumeration; skip it rather than fail the tick.
            let Ok(client) = x.inspect(window, &monitors, &struts) else {
                continue;
            };
            if client.desktop != current && client.desktop != ALL_DESKTOPS {
                continue;
            }
            result.identities.push(client.snapshot.id.clone());
            if !client.tileable || client.pid == Some(own) {
                continue;
            }
            result.windows.push(crate::TilingWindow {
                workspace: workspace_key(client.desktop),
                snapshot: client.snapshot,
                app: client.app,
                size_step: client.size_step,
            });
        }
        result.windows.sort_by_key(|w| w.snapshot.handle);
        Ok(result)
    })
}

/// The id tiling and positioning know an X11 window by.
pub fn window_id(handle: u64) -> Option<String> {
    let window = u32::try_from(handle).ok()?;
    with(|x| Ok(x.id(window))).ok()
}

/// Mark or unmark a docked window, so tiling in every tt-pane process skips it.
pub fn set_docked(handle: u64, docked: bool) -> Result<()> {
    let window = u32::try_from(handle)?;
    with(|x| {
        let atom = x.atom(DOCKED)?;
        if docked {
            x.connection.change_property32(
                PropMode::REPLACE,
                window,
                atom,
                AtomEnum::CARDINAL,
                &[1],
            )?;
        } else {
            x.connection.delete_property(window, atom)?;
        }
        x.connection.flush()?;
        Ok(())
    })
}

/// Let clicks fall through a parked window, or take them again. GNOME keeps a
/// corner of every window on screen, so a parked window still has one there.
pub fn set_click_through(handle: u64, through: bool) -> Result<()> {
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    let window = u32::try_from(handle)?;
    with(|x| {
        let parent = x.connection.query_tree(window)?.reply()?.parent;
        // A server-side decorated window takes clicks on its frame too.
        let windows = [window]
            .into_iter()
            .chain((parent != x.root).then_some(parent));
        for target in windows {
            if through {
                x.connection.shape_rectangles(
                    SO::SET,
                    SK::INPUT,
                    ClipOrdering::UNSORTED,
                    target,
                    0,
                    0,
                    &[],
                )?;
            } else {
                x.connection
                    .shape_mask(SO::SET, SK::INPUT, target, 0, 0, x11rb::NONE)?;
            }
        }
        x.connection.flush()?;
        Ok(())
    })
}

/// Publish the strip the dock reserves, or clear it. GNOME Shell reserves it
/// for its own chrome, which tiling here only sees through this property.
pub fn set_dock_strut(reserved: Option<(Rect, DockEdge)>) -> Result<()> {
    with(|x| {
        let atom = x.atom(DOCK_STRUT)?;
        let screen = &x.connection.setup().roots[x.screen];
        match reserved {
            Some((rect, edge)) => {
                let strut = strut_for(
                    rect,
                    edge,
                    i32::from(screen.width_in_pixels),
                    i32::from(screen.height_in_pixels),
                );
                x.connection.change_property32(
                    PropMode::REPLACE,
                    x.root,
                    atom,
                    AtomEnum::CARDINAL,
                    &strut,
                )?;
            }
            None => {
                x.connection.delete_property(x.root, atom)?;
            }
        }
        x.connection.flush()?;
        Ok(())
    })
}

/// `_NET_WM_STRUT_PARTIAL` reserving `rect` along `edge` of its monitor,
/// measured from the edges of the whole screen.
fn strut_for(rect: Rect, edge: DockEdge, root_w: i32, root_h: i32) -> Strut {
    let mut strut = [0u32; 12];
    let span_x = [
        rect.x.max(0) as u32,
        (rect.x + rect.width - 1).max(0) as u32,
    ];
    let span_y = [
        rect.y.max(0) as u32,
        (rect.y + rect.height - 1).max(0) as u32,
    ];
    match edge {
        DockEdge::Left => {
            strut[0] = (rect.x + rect.width).max(0) as u32;
            strut[4..6].copy_from_slice(&span_y);
        }
        DockEdge::Right => {
            strut[1] = (root_w - rect.x).max(0) as u32;
            strut[6..8].copy_from_slice(&span_y);
        }
        DockEdge::Top => {
            strut[2] = (rect.y + rect.height).max(0) as u32;
            strut[8..10].copy_from_slice(&span_x);
        }
        DockEdge::Bottom => {
            strut[3] = (root_h - rect.y).max(0) as u32;
            strut[10..12].copy_from_slice(&span_x);
        }
    }
    strut
}

fn workspace_key(desktop: u32) -> String {
    if desktop == ALL_DESKTOPS {
        "x11:all".to_owned()
    } else {
        format!("x11:{desktop}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect {
        x: 1920,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn struts_reserve_only_monitors_they_span() {
        // A 32 px top panel across the right monitor and a left dock on the left one.
        let top: Strut = [0, 0, 32, 0, 0, 0, 0, 0, 1920, 3839, 0, 0];
        let left: Strut = [48, 0, 0, 0, 0, 1079, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            work_area(MONITOR, &[top, left], 3840, 1080),
            Rect {
                y: 32,
                height: 1048,
                ..MONITOR
            }
        );
        assert_eq!(
            work_area(Rect { x: 0, ..MONITOR }, &[top, left], 3840, 1080),
            Rect {
                x: 48,
                y: 0,
                width: 1872,
                height: 1080,
            }
        );
    }

    #[test]
    fn window_manager_work_area_covers_shell_chrome() {
        // GNOME on X11: a 32 px top bar on a 4K monitor beside an offset 1080p one.
        let strut = workarea_strut([0, 32, 5760, 2128], 5760, 2160).unwrap();
        let primary = Rect {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
        };
        assert_eq!(
            work_area(primary, &[strut], 5760, 2160),
            Rect {
                y: 32,
                height: 2128,
                ..primary
            }
        );
        let secondary = Rect {
            x: 3840,
            y: 424,
            width: 1920,
            height: 1080,
        };
        assert_eq!(work_area(secondary, &[strut], 5760, 2160), secondary);
        assert_eq!(workarea_strut([0, 0, 0, 0], 5760, 2160), None);
    }

    #[test]
    fn sticky_windows_share_the_all_desktops_layout() {
        assert_eq!(workspace_key(2), "x11:2");
        assert_eq!(workspace_key(ALL_DESKTOPS), "x11:all");
    }
}
