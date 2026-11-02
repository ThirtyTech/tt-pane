use crate::geometry::{Action, Rect, monitor_for};
use anyhow::{Context, Result, ensure};
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::*},
    rust_connection::RustConnection,
};

struct X11 {
    connection: RustConnection,
    root: u32,
}
impl X11 {
    fn atom(&self, name: &str) -> Result<u32> {
        Ok(self
            .connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom)
    }
    fn property(&self, window: u32, name: &str) -> Result<Vec<u32>> {
        let reply = self
            .connection
            .get_property(false, window, self.atom(name)?, AtomEnum::ANY, 0, 4096)?
            .reply()?;
        Ok(reply.value32().map(|v| v.collect()).unwrap_or_default())
    }
    fn message(&self, window: u32, name: &str, data: [u32; 5]) -> Result<()> {
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
}

pub fn apply(action: Action) -> Result<()> {
    let (connection, screen) = x11rb::connect(None).context("Connect to X11")?;
    let root = connection.setup().roots[screen].root;
    let x = X11 { connection, root };
    let supported = x.property(root, "_NET_SUPPORTED")?;
    ensure!(
        supported.contains(&x.atom("_NET_MOVERESIZE_WINDOW")?),
        "Window manager does not support EWMH window positioning"
    );
    let window = *x
        .property(root, "_NET_ACTIVE_WINDOW")?
        .first()
        .context("No active X11 window")?;
    ensure!(
        window != 0 && window != root,
        "No active application window"
    );
    let geometry = x.connection.get_geometry(window)?.reply()?;
    let translated = x
        .connection
        .translate_coordinates(window, root, 0, 0)?
        .reply()?;
    let extents = x.property(window, "_NET_FRAME_EXTENTS")?;
    let edge = |i: usize| extents.get(i).copied().unwrap_or(0) as i32;
    let current = Rect {
        x: i32::from(translated.dst_x) - edge(0),
        y: i32::from(translated.dst_y) - edge(2),
        width: i32::from(geometry.width) + edge(0) + edge(1),
        height: i32::from(geometry.height) + edge(2) + edge(3),
    };
    let monitors: Vec<Rect> = x
        .connection
        .randr_get_monitors(root, true)?
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
    let monitor = monitors[monitor_for(current, &monitors).context("No RandR monitors found")?];
    let mut area = monitor;
    // Partial struts reserve only the monitors intersecting a panel's span.
    for client in x.property(root, "_NET_CLIENT_LIST")? {
        let s = x.property(client, "_NET_WM_STRUT_PARTIAL")?;
        if s.len() < 12 {
            continue;
        }
        let screen = &x.connection.setup().roots[screen];
        let root_w = i32::from(screen.width_in_pixels);
        let root_h = i32::from(screen.height_in_pixels);
        let spans_y = |start: u32, end: u32| {
            (start as i32) < monitor.y + monitor.height && end as i32 >= monitor.y
        };
        let spans_x = |start: u32, end: u32| {
            (start as i32) < monitor.x + monitor.width && end as i32 >= monitor.x
        };
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
    let target = action.target(area, current);
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
    x.message(
        window,
        "_NET_MOVERESIZE_WINDOW",
        [
            1 | (0xf << 8) | (2 << 12),
            target.x as u32,
            target.y as u32,
            (target.width - edge(0) - edge(1)).max(1) as u32,
            (target.height - edge(2) - edge(3)).max(1) as u32,
        ],
    )?;
    x.connection.flush()?;
    Ok(())
}
