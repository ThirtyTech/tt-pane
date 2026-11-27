//! The dock on GNOME, in X11 and Wayland sessions alike. tt-pane lays the panel
//! out here, and the bundled Shell extension draws it: live clones of the
//! docked windows in their slots, glides in and out of the dock, and the clicks
//! and drags on the panel, all over the session bus (see packaging/gnome).
//!
//! Windows are known by the handle the rest of tt-pane uses for them in the
//! session: the X window ID on X11, Mutter's window ID on Wayland.
//!
//! Mutter keeps part of every window on screen, so a parked window still has a
//! corner there, at the bottom of the stack. The extension makes it transparent
//! and lets clicks through it (on X11, tt-pane clears its input shape), while
//! its clone in the panel keeps showing it whole.
use super::{gnome, x11_session};
pub use crate::dock::{DockLayout, DockRequest};
use crate::{
    dock::{self, DockEdge, DockFill},
    geometry::Rect,
};
use anyhow::{Context, Result};
use std::{collections::BTreeMap, sync::Mutex};

type Wire = (i32, i32, i32, i32);

const NO_RECT: Wire = (0, 0, 0, 0);

fn wire(rect: Rect) -> Wire {
    (rect.x, rect.y, rect.width, rect.height)
}

fn rect((x, y, width, height): Wire) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn call<B, R>(method: &str, body: &B) -> Result<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    gnome::call(&gnome::connection()?, method, body)
}

/// What the panel was last asked to show, so an unchanged layout isn't sent again.
#[derive(Debug, Clone, PartialEq)]
struct Sent {
    edge: u32,
    panel: Wire,
    reserve: Wire,
    slots: Vec<(u64, Wire)>,
    drop: Wire,
    active: u64,
    highlight: bool,
    numbered: bool,
}

struct State {
    request: Option<DockRequest>,
    layout: Option<DockLayout>,
    sent: Option<Sent>,
    /// The strip the panel reserves, once its windows fill the edge.
    reserved: Option<Rect>,
    clicks: Vec<u64>,
    moves: Vec<(u64, usize)>,
    /// The window dock pick mode chose last, waiting for the next observation.
    picked: Option<u64>,
    picking: bool,
    /// Where Mutter put windows sent to each park spot, by spot: it keeps part
    /// of every window on screen, so they don't land exactly there.
    parks: BTreeMap<(i32, i32, i32, i32), Rect>,
}

static STATE: Mutex<State> = Mutex::new(State {
    request: None,
    layout: None,
    sent: None,
    reserved: None,
    clicks: Vec::new(),
    moves: Vec::new(),
    picked: None,
    picking: false,
    parks: BTreeMap::new(),
});

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The primary monitor, its work area, and every monitor.
fn monitors() -> Result<(Rect, Rect, Vec<Rect>)> {
    let (monitor, work, all): (Wire, Wire, Vec<Wire>) = call("DockMonitor", &())?;
    Ok((
        rect(monitor),
        rect(work),
        all.into_iter().map(rect).collect(),
    ))
}

/// `monitors` for the tray that shows the dock, which this makes the dock's
/// owner in the extension: if the tray goes away, such as after a crash, the
/// docked windows come back on screen.
///
/// A panel the extension no longer shows, though tt-pane showed one, means
/// GNOME Shell restarted and forgot the dock: the docked windows are marked
/// again here, and the next observation parks them out of sight again.
fn hold() -> Result<(Rect, Rect)> {
    let (monitor, work, shown): (Wire, Wire, bool) = call("DockHold", &())?;
    let forgotten = {
        let mut state = lock();
        if !shown && state.sent.take().is_some() {
            state.parks.clear();
            state.request.as_ref().map(|r| r.members.clone())
        } else {
            None
        }
    };
    for handle in forgotten.unwrap_or_default() {
        let _ = set_docked(handle, true);
    }
    Ok((rect(monitor), rect(work)))
}

/// The primary monitor's bounds and work area. Once the panel reserves its
/// strip, the work area excludes it.
pub fn dock_monitor() -> Result<(Rect, Rect)> {
    monitors().map(|(monitor, work, _)| (monitor, work))
}

/// `work` with the strip the panel reserves given back, so the panel is laid
/// out the same whether or not GNOME has applied the reservation yet.
fn without_reserved(work: Rect, reserved: Option<Rect>, edge: DockEdge) -> Rect {
    let Some(strip) = reserved else {
        return work;
    };
    let (mut left, mut top) = (work.x, work.y);
    let (mut right, mut bottom) = (work.x + work.width, work.y + work.height);
    match edge {
        DockEdge::Top => top = top.min(strip.y),
        DockEdge::Bottom => bottom = bottom.max(strip.y + strip.height),
        DockEdge::Left => left = left.min(strip.x),
        DockEdge::Right => right = right.max(strip.x + strip.width),
    }
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// The panel's layout for `request` on a monitor with `work` area (without
/// the panel's own reservation): the panel, the strip it reserves, and what
/// it shows.
fn lay_out(
    request: &DockRequest,
    monitor: Rect,
    work: Rect,
) -> Option<(DockLayout, Sent, Option<Rect>)> {
    let docked = request.members.len();
    let count = docked + usize::from(request.dragging);
    if count == 0 {
        return None;
    }
    let spacing = dock::Spacing::from_gaps(request.gaps);
    let strip = dock::strip_within(monitor, work, request.edge, request.percent);
    // Reserve space only once the docked windows fill the whole edge, as on
    // Windows; a drag never reserves, so other windows don't move mid-drag.
    let full = docked > 0 && dock::fills(strip, request.edge, docked, spacing);
    let reserve = full.then_some(strip);
    let panel = dock::occupied(strip, request.edge, request.fill, count, spacing);
    let rects = dock::slots_from(panel, request.edge, request.fill, count, spacing);
    let slots: Vec<(u64, Rect)> = request.members.iter().copied().zip(rects.clone()).collect();
    let drop = request
        .dragging
        .then(|| rects.get(docked).copied())
        .flatten();
    let area = (docked > 0).then(|| {
        if full {
            strip
        } else {
            // Snaps keep clear of what the previews show, not the panel's
            // full thickness.
            let used = dock::occupied(strip, request.edge, request.fill, docked, spacing);
            let shown: Vec<Rect> = slots.iter().map(|&(_, r)| r).collect();
            dock::shrink_to(used, request.edge, &shown)
        }
    });
    let sent = Sent {
        edge: request.edge as u32,
        panel: wire(panel),
        reserve: reserve.map_or(NO_RECT, |strip| wire(to_edge(strip, monitor, request.edge))),
        slots: slots.iter().map(|&(h, r)| (h, wire(r))).collect(),
        drop: drop.map_or(NO_RECT, wire),
        active: request.active.unwrap_or(0),
        highlight: request.highlight,
        numbered: request.numbered,
    };
    Some((DockLayout { panel, slots, area }, sent, reserve))
}

/// `strip` stretched out to `edge` of the monitor, past a top bar or another
/// dock that already reserves the space between: GNOME only takes a reserved
/// rectangle that touches the monitor's edge.
fn to_edge(strip: Rect, monitor: Rect, edge: DockEdge) -> Rect {
    let mut rect = strip;
    match edge {
        DockEdge::Top => {
            rect.height = strip.y + strip.height - monitor.y;
            rect.y = monitor.y;
        }
        DockEdge::Bottom => rect.height = monitor.y + monitor.height - strip.y,
        DockEdge::Left => {
            rect.width = strip.x + strip.width - monitor.x;
            rect.x = monitor.x;
        }
        DockEdge::Right => rect.width = monitor.x + monitor.width - strip.x,
    }
    rect
}

/// Show the panel, or update what it shows.
pub fn show_dock(request: DockRequest) -> Result<()> {
    lock().request = Some(request);
    render()
}

fn render() -> Result<()> {
    let (request, reserved) = {
        let state = lock();
        let Some(request) = state.request.clone() else {
            return Ok(());
        };
        (request, state.reserved)
    };
    let (monitor, work) = hold()?;
    let work = without_reserved(work, reserved, request.edge);
    let Some((layout, sent, reserve)) = lay_out(&request, monitor, work) else {
        // Nothing docked or dragged: hide and give the space back.
        if lock().sent.take().is_some() || reserved.is_some() {
            call::<_, ()>("DockHidePanel", &())?;
        }
        publish_strut(None, request.edge);
        let mut state = lock();
        state.reserved = None;
        state.layout = None;
        return Ok(());
    };
    if lock().sent.as_ref() != Some(&sent) {
        call::<_, ()>(
            "DockPanel",
            &(
                sent.edge,
                sent.panel,
                sent.reserve,
                sent.slots.clone(),
                sent.drop,
                sent.active,
                sent.highlight,
                sent.numbered,
            ),
        )?;
    }
    if reserve != reserved {
        publish_strut(reserve, request.edge);
    }
    let mut state = lock();
    state.sent = Some(sent);
    state.reserved = reserve;
    state.layout = Some(layout);
    Ok(())
}

/// Tiling on X11 reads struts from X properties, which GNOME doesn't set for
/// its own chrome; publish the dock's.
fn publish_strut(reserve: Option<Rect>, edge: DockEdge) {
    if x11_session() {
        let _ = super::x11::set_dock_strut(reserve.map(|r| (r, edge)));
    }
}

/// Remove the panel and give its space back to the work area.
pub fn hide_dock() {
    let had = {
        let mut state = lock();
        state.request = None;
        state.layout = None;
        state.clicks.clear();
        state.moves.clear();
        state.picked = None;
        state.sent.take().is_some() || state.reserved.take().is_some()
    };
    if had {
        let _ = call::<_, ()>("DockHidePanel", &());
        publish_strut(None, DockEdge::default());
    }
}

/// The panel and slot rectangles last laid out, while the panel is shown.
pub fn dock_layout() -> Option<DockLayout> {
    lock().layout.clone()
}

/// Previews clicked, and previews moved with their new places.
type Events = (Vec<u64>, Vec<(u64, u32)>);

/// Collect the clicks and moves made on the panel since the last call.
fn fetch_events() {
    let Ok((clicks, moves)): Result<Events> = call("DockEvents", &()) else {
        return;
    };
    let mut state = lock();
    state.clicks.extend(clicks);
    state
        .moves
        .extend(moves.into_iter().map(|(h, to)| (h, to as usize)));
}

/// Windows whose previews were clicked since the last call, then the one dock
/// pick mode chose, which counts as the latest click.
pub fn take_dock_clicks() -> Vec<u64> {
    fetch_events();
    let mut state = lock();
    let mut clicks = std::mem::take(&mut state.clicks);
    clicks.extend(state.picked.take());
    clicks
}

/// Previews dragged to a new place since the last call: each window and its
/// new place in the dock. `take_dock_clicks` collects them.
pub fn take_dock_moves() -> Vec<(u64, usize)> {
    std::mem::take(&mut lock().moves)
}

/// Bring `handle` out of the dock at the next observation, as if its preview
/// were clicked. A newer pick replaces one still waiting.
pub fn pick_dock(handle: u64) {
    lock().picked = Some(handle);
}

/// The pick still waiting for the next observation.
pub fn pending_dock_pick() -> Option<u64> {
    lock().picked
}

/// Turn the numbered previews of dock pick mode on or off, right away.
pub fn set_dock_picking(on: bool) {
    let changed = {
        let mut state = lock();
        let changed = state.picking != on;
        state.picking = on;
        if let Some(request) = &mut state.request {
            request.numbered = on;
        }
        changed
    };
    if changed {
        let _ = render();
    }
}

/// Whether dock pick mode is on.
pub fn dock_picking() -> bool {
    lock().picking
}

/// Whether a tt-pane tray shows the dock in this session.
pub fn dock_running() -> bool {
    call::<_, (u64, (i32, i32), bool)>("DockState", &()).is_ok_and(|(_, _, running)| running)
}

/// The window being moved with the mouse right now, if any.
pub fn moving_window() -> Option<u64> {
    call::<_, (u64, (i32, i32), bool)>("DockState", &())
        .ok()
        .map(|(moving, _, _)| moving)
        .filter(|&moving| moving != 0)
}

/// The mouse position in screen pixels.
pub fn cursor_position() -> Option<(i32, i32)> {
    call::<_, (u64, (i32, i32), bool)>("DockState", &())
        .ok()
        .map(|(_, pointer, _)| pointer)
}

/// The id tiling and positioning know a window by in this session.
fn window_id(handle: u64) -> Option<String> {
    if x11_session() {
        super::x11::window_id(handle)
    } else {
        Some(format!("gnome:{handle}"))
    }
}

/// The id and visible bounds of a window that still exists, docked or not.
pub fn docked_window(handle: u64) -> Option<(String, Rect)> {
    let (exists, frame, _): (bool, Wire, bool) = call("DockWindow", &(handle,)).ok()?;
    exists.then(|| Some((window_id(handle)?, rect(frame))))?
}

pub fn is_minimized(handle: u64) -> bool {
    call::<_, (bool, Wire, bool)>("DockWindow", &(handle,))
        .is_ok_and(|(exists, _, minimized)| exists && minimized)
}

/// Mark or unmark a window as docked. Marked windows are invisible to tiling,
/// centered groups and directional focus, in every tt-pane process.
pub fn set_docked(handle: u64, docked: bool) -> Result<()> {
    call::<_, ()>("DockMark", &(handle, docked))?;
    if x11_session() {
        super::x11::set_docked(handle, docked)?;
        if !docked {
            super::x11::set_click_through(handle, false)?;
        }
    }
    Ok(())
}

pub fn activate_handle(handle: u64) -> Result<()> {
    call::<_, ()>("DockActivate", &(handle,))
}

/// Move a window, gliding there when `with_snap_animation` asks for it, and
/// show it if it was parked.
pub fn place_window(handle: u64, target: Rect) -> Result<Rect> {
    if let Some(duration) = crate::snap_animation()
        && let Some((_, from)) = docked_window(handle)
    {
        crate::animation::run(
            from,
            target,
            duration,
            &mut |step| call::<_, Wire>("DockPlace", &(handle, wire(step), false)).map(|_| ()),
            &mut crate::animation::pause,
        );
    }
    let placed = call::<_, Wire>("DockPlace", &(handle, wire(target), false))?;
    if x11_session() {
        super::x11::set_click_through(handle, false)?;
    }
    Ok(rect(placed))
}

/// Where a docked window of `size` at `step` is sent: past the bottom corner
/// of the dock's monitor on its fill side, so only a corner can stay on
/// screen. Mutter keeps a window's top edge on screen, so windows always park
/// at the bottom. A corner that touches another monitor gives way to the other one.
fn park_spot(
    monitor: Rect,
    others: &[Rect],
    edge: DockEdge,
    fill: DockFill,
    size: (i32, i32),
    step: usize,
) -> Rect {
    let right = match edge {
        DockEdge::Right => true,
        DockEdge::Left => false,
        DockEdge::Top | DockEdge::Bottom => fill == DockFill::End,
    };
    // Bottom-left or bottom-right, one of which park_rect takes from the edge and fill.
    let corner = |right: bool| {
        let (edge, fill) = if right {
            (DockEdge::Right, DockFill::End)
        } else {
            (DockEdge::Left, DockFill::End)
        };
        dock::park_rect(monitor, edge, fill, size, step, others)
    };
    corner(right).or_else(|| corner(!right)).unwrap_or_else(|| {
        // Every corner touches another monitor: sit past the primary's bottom
        // corner anyway. The window is transparent and lets clicks through.
        dock::park_rect(monitor, DockEdge::Left, DockFill::End, size, step, &[]).unwrap_or(monitor)
    })
}

fn park_target(edge: DockEdge, fill: DockFill, size: (i32, i32), step: usize) -> Result<Rect> {
    let (monitor, _, all) = monitors()?;
    let others: Vec<Rect> = all.into_iter().filter(|m| *m != monitor).collect();
    Ok(park_spot(monitor, &others, edge, fill, size, step))
}

fn spot_key(spot: Rect) -> (i32, i32, i32, i32) {
    wire(spot)
}

/// Whether a docked window at `bounds` is parked for `step`: at its spot, where
/// Mutter put a window sent there, or anywhere past the same bottom corner with
/// no more on screen than Mutter insists on. A Wayland window resizes after it
/// moves, so for a moment it sits at the spot for its old size.
pub fn dock_parked(edge: DockEdge, fill: DockFill, bounds: Rect, _live: bool, step: usize) -> bool {
    let Ok((monitor, _, all)) = monitors() else {
        return false;
    };
    let others: Vec<Rect> = all.into_iter().filter(|m| *m != monitor).collect();
    let spot = park_spot(
        monitor,
        &others,
        edge,
        fill,
        (bounds.width, bounds.height),
        step,
    );
    let near = |a: Rect| a.x.abs_diff(bounds.x) <= 2 && a.y.abs_diff(bounds.y) <= 2;
    near(spot)
        || lock()
            .parks
            .get(&spot_key(spot))
            .is_some_and(|landed| near(*landed))
        || past_corner(bounds, spot, monitor)
}

/// Mutter keeps at least this much of a window on screen across, and a
/// window's title bar down: tt-pane allows a little more.
const ON_SCREEN_ACROSS: i32 = 160;
const ON_SCREEN_DOWN: i32 = 240;

/// Whether `bounds` hangs off the same bottom corner of `monitor` as `spot`,
/// with only a corner left on it.
fn past_corner(bounds: Rect, spot: Rect, monitor: Rect) -> bool {
    let left = bounds.x.max(monitor.x);
    let right = (bounds.x + bounds.width).min(monitor.x + monitor.width);
    let top = bounds.y.max(monitor.y);
    let bottom = (bounds.y + bounds.height).min(monitor.y + monitor.height);
    let (across, down) = (right - left, bottom - top);
    let off_left = bounds.x < monitor.x;
    let off_right = bounds.x + bounds.width > monitor.x + monitor.width;
    let same_side = if spot.x < monitor.x {
        off_left
    } else {
        off_right
    };
    same_side
        && bounds.y + bounds.height > monitor.y + monitor.height
        && (1..=ON_SCREEN_ACROSS).contains(&across)
        && (1..=ON_SCREEN_DOWN).contains(&down)
}

fn remember_park(spot: Rect, landed: Rect) {
    lock().parks.insert(spot_key(spot), landed);
}

/// Park a docked window at 1080p. GNOME's clones show a window at its own
/// size, so there is no need to lay it out on screen first. Previews are
/// always live: a clone keeps a window drawing, wherever it is.
pub fn park_window(
    handle: u64,
    edge: DockEdge,
    fill: DockFill,
    _live: bool,
    step: usize,
) -> Result<Rect> {
    let spot = park_target(edge, fill, dock::PREVIEW_SOURCE, step)?;
    let landed = rect(call::<_, Wire>("DockPlace", &(handle, wire(spot), true))?);
    remember_park(spot, landed);
    if x11_session() {
        super::x11::set_click_through(handle, true)?;
    }
    Ok(landed)
}

/// Send a window in the arena back to the dock at its current size. With
/// animation on and a `slot`, a live clone glides into the slot.
pub fn stow_window(
    handle: u64,
    slot: Option<Rect>,
    edge: DockEdge,
    fill: DockFill,
    _live: bool,
    step: usize,
) -> Result<Rect> {
    let (_, bounds) = docked_window(handle).context("Window is no longer available")?;
    let spot = park_target(edge, fill, (bounds.width, bounds.height), step)?;
    let duration = crate::snap_animation().map_or(0, |d| d.as_millis().min(10_000) as u32);
    let landed = rect(call::<_, Wire>(
        "DockStow",
        &(handle, wire(spot), slot.map_or(NO_RECT, wire), duration),
    )?);
    remember_park(spot, landed);
    if x11_session() {
        super::x11::set_click_through(handle, true)?;
    }
    Ok(landed)
}

/// Bring a docked window from its preview `slot` to `target`. With animation
/// on, a live clone grows out of the slot and the window takes its place at
/// the end; the extension runs the glide, so this returns straight away.
pub fn lift_window(handle: u64, slot: Rect, target: Rect) -> Result<Rect> {
    let duration = crate::snap_animation().map_or(0, |d| d.as_millis().min(10_000) as u32);
    call::<_, ()>("DockLift", &(handle, wire(slot), wire(target), duration))?;
    if x11_session() {
        super::x11::set_click_through(handle, false)?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const WORK: Rect = Rect {
        x: 0,
        y: 32,
        width: 1920,
        height: 1048,
    };

    fn request(members: Vec<u64>) -> DockRequest {
        DockRequest {
            edge: DockEdge::Bottom,
            fill: DockFill::Start,
            percent: 10,
            members,
            active: None,
            dragging: false,
            highlight: false,
            numbered: false,
            gaps: crate::geometry::SnapGaps { inner: 4, outer: 8 },
        }
    }

    #[test]
    fn an_empty_dock_has_no_panel_and_a_drag_offers_a_slot() {
        assert!(lay_out(&request(vec![]), MONITOR, WORK).is_none());
        let dragging = DockRequest {
            dragging: true,
            ..request(vec![])
        };
        let (layout, sent, reserve) = lay_out(&dragging, MONITOR, WORK).unwrap();
        assert!(layout.slots.is_empty());
        assert_eq!(layout.area, None);
        assert_ne!(sent.drop, NO_RECT);
        assert_eq!(reserve, None);
    }

    #[test]
    fn a_short_panel_reserves_nothing_and_a_full_one_reserves_its_strip() {
        let (layout, sent, reserve) = lay_out(&request(vec![1, 2]), MONITOR, WORK).unwrap();
        assert_eq!(reserve, None);
        assert_eq!(sent.reserve, NO_RECT);
        assert_eq!(layout.slots.len(), 2);
        // Along the bottom, from the left.
        assert_eq!(layout.panel.x, 0);
        assert_eq!(layout.panel.y + layout.panel.height, 1080);
        assert!(layout.panel.width < 1920);
        let many: Vec<u64> = (1..=20).collect();
        let top = DockRequest {
            edge: DockEdge::Top,
            ..request(many.clone())
        };
        let (layout, sent, reserve) = lay_out(&request(many), MONITOR, WORK).unwrap();
        let strip = reserve.unwrap();
        assert_eq!(layout.area, Some(strip));
        assert_eq!((strip.width, strip.y + strip.height), (1920, 1080));
        assert_eq!(sent.reserve, wire(strip));
        // Below the top bar, the reservation reaches up to the monitor's edge.
        let (_, sent, reserve) = lay_out(&top, MONITOR, WORK).unwrap();
        let strip = reserve.unwrap();
        assert_eq!(strip.y, 32);
        assert_eq!(sent.reserve, (0, 0, 1920, strip.height + 32));
    }

    #[test]
    fn the_reserved_strip_is_given_back_before_laying_out_again() {
        let strip = Rect {
            x: 0,
            y: 972,
            width: 1920,
            height: 108,
        };
        let reduced = Rect {
            height: 940,
            ..WORK
        };
        assert_eq!(
            without_reserved(reduced, Some(strip), DockEdge::Bottom),
            WORK
        );
        assert_eq!(without_reserved(WORK, Some(strip), DockEdge::Bottom), WORK);
        assert_eq!(without_reserved(reduced, None, DockEdge::Bottom), reduced);
    }

    #[test]
    fn a_window_mutter_left_a_corner_of_counts_as_parked() {
        let spot = park_spot(
            MONITOR,
            &[],
            DockEdge::Bottom,
            DockFill::Start,
            (1920, 1080),
            0,
        );
        // Mutter keeps 75 px across and the title bar down on screen.
        let landed = Rect {
            x: -1845,
            y: 967,
            width: 1920,
            height: 1080,
        };
        assert!(past_corner(landed, spot, MONITOR));
        // Still its old size, before a Wayland client catches up.
        let small = Rect {
            x: -725,
            y: 929,
            width: 800,
            height: 637,
        };
        assert!(past_corner(small, spot, MONITOR));
        // The other corner, or out on the screen, doesn't.
        assert!(!past_corner(Rect { x: 1845, ..landed }, spot, MONITOR));
        assert!(!past_corner(
            Rect {
                x: 100,
                y: 100,
                ..small
            },
            spot,
            MONITOR
        ));
    }

    #[test]
    fn windows_park_past_a_bottom_corner_away_from_other_monitors() {
        let size = (1920, 1080);
        let left = park_spot(MONITOR, &[], DockEdge::Bottom, DockFill::Start, size, 0);
        assert_eq!(
            (left.x + left.width, left.y),
            (dock::PARK_SLIVER, 1080 - dock::PARK_SLIVER)
        );
        let right = park_spot(MONITOR, &[], DockEdge::Top, DockFill::End, size, 0);
        assert_eq!(
            (right.x, right.y),
            (1920 - dock::PARK_SLIVER, 1080 - dock::PARK_SLIVER)
        );
        // A monitor to the right takes the other corner.
        let beside = Rect { x: 1920, ..MONITOR };
        let away = park_spot(
            MONITOR,
            &[beside],
            DockEdge::Right,
            DockFill::Start,
            size,
            0,
        );
        assert_eq!(away.x + away.width, dock::PARK_SLIVER);
    }
}
