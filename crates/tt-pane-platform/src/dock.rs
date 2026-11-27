//! Portable dock layout: the panel along one monitor edge, the arena it leaves,
//! and the 16:9 preview slots inside the panel.
use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

/// The screen edge the dock panel sits on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Top,
    Left,
    Right,
    #[default]
    Bottom,
}

impl DockEdge {
    pub const ALL: [Self; 4] = [Self::Top, Self::Left, Self::Right, Self::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// The end of the edge the panel grows from as windows are docked: the left or
/// top end (`Start`), or the right or bottom end (`End`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockFill {
    #[default]
    Start,
    End,
}

impl DockFill {
    pub const ALL: [Self; 2] = [Self::Start, Self::End];

    /// The end's name along `edge`: left/right on top and bottom, top/bottom on the sides.
    pub fn label(self, edge: DockEdge) -> &'static str {
        match (self, edge.horizontal()) {
            (Self::Start, true) => "Left",
            (Self::End, true) => "Right",
            (Self::Start, false) => "Top",
            (Self::End, false) => "Bottom",
        }
    }
}

/// The size docked windows are rendered at before they are scaled into a slot.
pub const PREVIEW_SOURCE: (i32, i32) = (1920, 1080);

/// Spacing between slots and around them, in physical pixels.
const PADDING: i32 = 12;
/// Previews never shrink below this width, even when many windows are docked.
const MIN_SLOT_WIDTH: i32 = 48;

/// The strip along `edge` that takes `percent` of the monitor's height (top and
/// bottom) or width (left and right).
pub fn panel_rect(monitor: Rect, edge: DockEdge, percent: u32) -> Rect {
    let percent = percent.clamp(1, 90) as i64;
    let mut panel = monitor;
    if edge.horizontal() {
        panel.height = ((i64::from(monitor.height) * percent) / 100).max(1) as i32;
        if edge == DockEdge::Bottom {
            panel.y = monitor.y + monitor.height - panel.height;
        }
    } else {
        panel.width = ((i64::from(monitor.width) * percent) / 100).max(1) as i32;
        if edge == DockEdge::Right {
            panel.x = monitor.x + monitor.width - panel.width;
        }
    }
    panel
}

/// What remains of `area` once the panel on `edge` is taken out of it.
pub fn arena(area: Rect, panel: Rect, edge: DockEdge) -> Rect {
    let mut arena = area;
    match edge {
        DockEdge::Top => {
            let bottom = arena.y + arena.height;
            arena.y = arena.y.max(panel.y + panel.height);
            arena.height = (bottom - arena.y).max(1);
        }
        DockEdge::Bottom => {
            arena.height = (arena.height.min(panel.y - arena.y)).max(1);
        }
        DockEdge::Left => {
            let right = arena.x + arena.width;
            arena.x = arena.x.max(panel.x + panel.width);
            arena.width = (right - arena.x).max(1);
        }
        DockEdge::Right => {
            arena.width = (arena.width.min(panel.x - arena.x)).max(1);
        }
    }
    arena
}

/// `count` 16:9 slots laid out along the panel and centered on it. Slots shrink
/// so they all fit; past the minimum size the last ones are clipped by the panel.
pub fn slots(panel: Rect, edge: DockEdge, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i64;
    let (source_w, source_h) = (i64::from(PREVIEW_SOURCE.0), i64::from(PREVIEW_SOURCE.1));
    let pad = i64::from(PADDING);
    let (along, across) = if edge.horizontal() {
        (i64::from(panel.width), i64::from(panel.height))
    } else {
        (i64::from(panel.height), i64::from(panel.width))
    };
    let thickness = (across - 2 * pad).max(1);
    // Size of a slot along the panel when it fills the panel's thickness.
    let natural = if edge.horizontal() {
        thickness * source_w / source_h
    } else {
        thickness * source_h / source_w
    };
    let fit = ((along - pad * (n + 1)) / n).max(1);
    let min_length = if edge.horizontal() {
        i64::from(MIN_SLOT_WIDTH)
    } else {
        i64::from(MIN_SLOT_WIDTH) * source_h / source_w
    };
    let (length, cross) = if natural <= fit {
        (natural, thickness)
    } else {
        let length = fit.max(min_length);
        let cross = if edge.horizontal() {
            length * source_h / source_w
        } else {
            length * source_w / source_h
        };
        (length, cross)
    };
    let total = n * length + (n - 1) * pad;
    let start = (along - total) / 2;
    let start = start.max(pad);
    let offset_across = (across - cross) / 2;
    (0..n)
        .map(|i| {
            let along_pos = start + i * (length + pad);
            if edge.horizontal() {
                Rect {
                    x: panel.x + along_pos as i32,
                    y: panel.y + offset_across as i32,
                    width: length as i32,
                    height: cross as i32,
                }
            } else {
                Rect {
                    x: panel.x + offset_across as i32,
                    y: panel.y + along_pos as i32,
                    width: cross as i32,
                    height: length as i32,
                }
            }
        })
        .collect()
}

/// The strip a panel with `percent` of `monitor` would take along `edge` of
/// `area`, such as the work area beside a taskbar.
pub fn strip_within(monitor: Rect, area: Rect, edge: DockEdge, percent: u32) -> Rect {
    let full = panel_rect(monitor, edge, percent);
    let mut strip = area;
    match edge {
        DockEdge::Top => strip.height = full.height,
        DockEdge::Bottom => {
            strip.y = area.y + area.height - full.height;
            strip.height = full.height;
        }
        DockEdge::Left => strip.width = full.width,
        DockEdge::Right => {
            strip.x = area.x + area.width - full.width;
            strip.width = full.width;
        }
    }
    strip
}

/// The part of `strip` that `count` full-size slots need, starting from the
/// `fill` end. Past the strip's length the whole strip is used and slots shrink.
pub fn occupied(strip: Rect, edge: DockEdge, fill: DockFill, count: usize) -> Rect {
    let pad = i64::from(PADDING);
    let (along, across) = if edge.horizontal() {
        (i64::from(strip.width), i64::from(strip.height))
    } else {
        (i64::from(strip.height), i64::from(strip.width))
    };
    let thickness = (across - 2 * pad).max(1);
    let (source_w, source_h) = (i64::from(PREVIEW_SOURCE.0), i64::from(PREVIEW_SOURCE.1));
    let natural = if edge.horizontal() {
        thickness * source_w / source_h
    } else {
        thickness * source_h / source_w
    };
    let n = count as i64;
    let length = if n == 0 {
        0
    } else {
        (n * natural + (n + 1) * pad).clamp(0, along) as i32
    };
    let mut rect = strip;
    if edge.horizontal() {
        rect.width = length;
        if fill == DockFill::End {
            rect.x = strip.x + strip.width - length;
        }
    } else {
        rect.height = length;
        if fill == DockFill::End {
            rect.y = strip.y + strip.height - length;
        }
    }
    rect
}

/// Whether `count` full-size slots need the whole length of `strip`.
pub fn fills(strip: Rect, edge: DockEdge, count: usize) -> bool {
    let used = occupied(strip, edge, DockFill::Start, count);
    if edge.horizontal() {
        used.width >= strip.width
    } else {
        used.height >= strip.height
    }
}

/// Slots in fill order: the first docked window sits at the `fill` end.
pub fn slots_from(panel: Rect, edge: DockEdge, fill: DockFill, count: usize) -> Vec<Rect> {
    let mut slots = slots(panel, edge, count);
    if fill == DockFill::End {
        slots.reverse();
    }
    slots
}

/// How much of a parked window stays on the monitor, in each direction.
pub const PARK_SLIVER: i32 = 8;

/// How far each docked window's park spot moves from the previous one's, along
/// both axes. Chrome stops drawing a window that others cover completely, even
/// off screen, so docked windows must not park on top of each other. Stepping
/// diagonally leaves every window one corner no other docked window covers.
pub const PARK_STEP: i32 = 2;

/// Docked windows past this many share the last park spot.
pub const PARK_STEPS: usize = 32;

/// How far the park spot for the docked window at `step` moves inward.
fn park_offset(step: usize) -> i32 {
    PARK_STEP * step.min(PARK_STEPS - 1) as i32
}

/// Where to park a docked window of `size`: nearly off the monitor past the
/// panel's fill corner, with a `PARK_SLIVER` square left on screen under the
/// panel. Chrome (and other Chromium apps) treat a window that is on no screen
/// at all as hidden and stop drawing it, which freezes its preview; a covered
/// sliver keeps them drawing. Each `step` (the window's place in the dock)
/// moves the spot `PARK_STEP` further onto the monitor, still under the panel.
/// `None` when that spot would show part of the window on one of the `others`
/// monitors.
pub fn park_rect(
    monitor: Rect,
    edge: DockEdge,
    fill: DockFill,
    (width, height): (i32, i32),
    step: usize,
    others: &[Rect],
) -> Option<Rect> {
    // The monitor corner at the fill end of the edge.
    let (right, bottom) = match (edge, fill) {
        (DockEdge::Top, DockFill::Start) | (DockEdge::Left, DockFill::Start) => (false, false),
        (DockEdge::Top, DockFill::End) | (DockEdge::Right, DockFill::Start) => (true, false),
        (DockEdge::Bottom, DockFill::Start) | (DockEdge::Left, DockFill::End) => (false, true),
        (DockEdge::Bottom, DockFill::End) | (DockEdge::Right, DockFill::End) => (true, true),
    };
    let offset = park_offset(step);
    let rect = Rect {
        x: if right {
            monitor.x + monitor.width - PARK_SLIVER - offset
        } else {
            monitor.x - width + PARK_SLIVER + offset
        },
        y: if bottom {
            monitor.y + monitor.height - PARK_SLIVER - offset
        } else {
            monitor.y - height + PARK_SLIVER + offset
        },
        width,
        height,
    };
    (!others.iter().any(|other| intersects(rect, *other))).then_some(rect)
}

/// Where to park a docked window of `size` when the panel's corner touches
/// another display: peeking `PARK_SLIVER` into a gap of the virtual screen (the
/// box around all `monitors`) where no monitor is, so nothing of it shows.
/// Chrome counts a window anywhere in that box as on screen and keeps drawing.
/// Each `step` moves the spot `PARK_STEP` further in, as in `park_rect`.
/// `None` when the monitors leave no such gap along the box's edges.
pub fn gap_park_rect(monitors: &[Rect], (width, height): (i32, i32), step: usize) -> Option<Rect> {
    let first = monitors.first()?;
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (first.x + first.width, first.y + first.height);
    for m in monitors {
        left = left.min(m.x);
        top = top.min(m.y);
        right = right.max(m.x + m.width);
        bottom = bottom.max(m.y + m.height);
    }
    let virtual_screen = Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    // Along the top and bottom, try spots beside each monitor and at the box's
    // ends; the same down the sides.
    let xs: Vec<i32> = monitors
        .iter()
        .flat_map(|m| [m.x - width, m.x + m.width])
        .chain([left, right - width])
        .collect();
    let ys: Vec<i32> = monitors
        .iter()
        .flat_map(|m| [m.y - height, m.y + m.height])
        .chain([top, bottom - height])
        .collect();
    // Each spot moves inward by the step's offset, and along the edge too.
    let o = park_offset(step);
    let along = |y: i32| xs.iter().map(move |&x| (x + o, y));
    let down = |x: i32| ys.iter().map(move |&y| (x, y + o));
    along(top - height + PARK_SLIVER + o)
        .chain(along(bottom - PARK_SLIVER - o))
        .chain(down(left - width + PARK_SLIVER + o))
        .chain(down(right - PARK_SLIVER - o))
        .map(|(x, y)| Rect {
            x,
            y,
            width,
            height,
        })
        .find(|rect| {
            intersects(*rect, virtual_screen) && !monitors.iter().any(|m| intersects(*rect, *m))
        })
}

/// Where a window grows to the preview's source size before it is parked: at
/// its own top-left corner, pulled back inside `area` so all of it is on
/// screen. Explorer, for one, only lays out the part of its file list that is
/// on a monitor, so a window resized off screen keeps a blank list.
pub fn staging_rect(current: Rect, area: Rect) -> Rect {
    let (width, height) = PREVIEW_SOURCE;
    let clamp =
        |pos: i32, start: i32, room: i32, size: i32| pos.min(start + room - size).max(start);
    Rect {
        x: clamp(current.x, area.x, area.width, width),
        y: clamp(current.y, area.y, area.height, height),
        width,
        height,
    }
}

/// Whether two rectangles share any area.
pub fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// The space between a slot's edge and the preview drawn in it. The panel is
/// transparent, so this shows as part of the gap beside a snapped window.
pub const SLOT_PADDING: u32 = 3;

/// How far a snap stops short of the panel: the same spacing as between two
/// snapped windows (each inset by `inner`), counting the slot padding that
/// already shows between the panel's edge and its previews.
pub fn snap_gap(inner: u32) -> u32 {
    inner.saturating_mul(2).saturating_sub(SLOT_PADDING)
}

/// The middle of `source` with the aspect ratio of a `width` × `height`
/// destination, so a preview fills its slot without stretching, whatever shape
/// the window has. A wide window loses a little of its sides, a tall one a
/// little of its top and bottom.
pub fn crop_to_aspect(source: Rect, width: i32, height: i32) -> Rect {
    if source.width <= 0 || source.height <= 0 || width <= 0 || height <= 0 {
        return source;
    }
    let (sw, sh) = (i64::from(source.width), i64::from(source.height));
    let (w, h) = (i64::from(width), i64::from(height));
    if sw * h > w * sh {
        let cropped = (sh * w / h).max(1);
        Rect {
            x: source.x + ((sw - cropped) / 2) as i32,
            width: cropped as i32,
            ..source
        }
    } else {
        let cropped = (sw * h / w).max(1);
        Rect {
            y: source.y + ((sh - cropped) / 2) as i32,
            height: cropped as i32,
            ..source
        }
    }
}

/// `area` with its inner side (the one facing the middle of the screen) pulled
/// back to `pad` past the furthest of the `previews`. Windows wider or taller
/// than their slot leave the rest of the slot empty, and snaps beside the panel
/// should close up to what's shown. Unchanged with no previews.
pub fn shrink_to(area: Rect, edge: DockEdge, previews: &[Rect], pad: u32) -> Rect {
    let pad = pad as i32;
    let mut out = area;
    match edge {
        DockEdge::Top => {
            if let Some(bottom) = previews.iter().map(|p| p.y + p.height).max() {
                out.height = (bottom + pad - area.y).clamp(1, area.height);
            }
        }
        DockEdge::Bottom => {
            if let Some(top) = previews.iter().map(|p| p.y).min() {
                out.y = (top - pad).clamp(area.y, area.y + area.height - 1);
                out.height = area.y + area.height - out.y;
            }
        }
        DockEdge::Left => {
            if let Some(right) = previews.iter().map(|p| p.x + p.width).max() {
                out.width = (right + pad - area.x).clamp(1, area.width);
            }
        }
        DockEdge::Right => {
            if let Some(left) = previews.iter().map(|p| p.x).min() {
                out.x = (left - pad).clamp(area.x, area.x + area.width - 1);
                out.width = area.x + area.width - out.x;
            }
        }
    }
    out
}

/// `rect` cut back so it stops `gap` short of the panel on `edge`, when the two
/// overlap. A rectangle that doesn't touch the panel is returned unchanged, so a
/// short panel only squeezes the windows it actually covers.
pub fn avoid(rect: Rect, panel: Rect, edge: DockEdge, gap: u32) -> Rect {
    if !intersects(rect, panel) {
        return rect;
    }
    let gap = gap.min(1000) as i32;
    let mut out = rect;
    match edge {
        DockEdge::Top => {
            let bottom = rect.y + rect.height;
            out.y = rect.y.max(panel.y + panel.height + gap);
            out.height = (bottom - out.y).max(1);
        }
        DockEdge::Bottom => {
            out.height = (rect.height.min(panel.y - gap - rect.y)).max(1);
        }
        DockEdge::Left => {
            let right = rect.x + rect.width;
            out.x = rect.x.max(panel.x + panel.width + gap);
            out.width = (right - out.x).max(1);
        }
        DockEdge::Right => {
            out.width = (rect.width.min(panel.x - gap - rect.x)).max(1);
        }
    }
    out
}

/// The index of the slot containing `point`.
pub fn hit_test(slots: &[Rect], point: (i32, i32)) -> Option<usize> {
    slots.iter().position(|slot| contains(*slot, point))
}

pub fn contains(rect: Rect, (x, y): (i32, i32)) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
    };

    #[test]
    fn panels_take_a_percentage_of_the_matching_dimension() {
        assert_eq!(
            panel_rect(MONITOR, DockEdge::Bottom, 15),
            Rect {
                x: 0,
                y: 1224,
                width: 2560,
                height: 216
            }
        );
        assert_eq!(
            panel_rect(MONITOR, DockEdge::Top, 15),
            Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 216
            }
        );
        assert_eq!(
            panel_rect(MONITOR, DockEdge::Left, 10),
            Rect {
                x: 0,
                y: 0,
                width: 256,
                height: 1440
            }
        );
        assert_eq!(
            panel_rect(MONITOR, DockEdge::Right, 10),
            Rect {
                x: 2304,
                y: 0,
                width: 256,
                height: 1440
            }
        );
    }

    #[test]
    fn the_arena_is_what_the_panel_leaves_of_the_work_area() {
        // A taskbar already takes 48 px off the bottom of the work area.
        let area = Rect {
            height: 1392,
            ..MONITOR
        };
        for edge in DockEdge::ALL {
            let panel = panel_rect(MONITOR, edge, 15);
            let arena = arena(area, panel, edge);
            let overlap = arena.x < panel.x + panel.width
                && panel.x < arena.x + arena.width
                && arena.y < panel.y + panel.height
                && panel.y < arena.y + arena.height;
            assert!(!overlap, "{edge:?}: {arena:?} overlaps {panel:?}");
        }
        assert_eq!(
            arena(area, panel_rect(MONITOR, DockEdge::Top, 15), DockEdge::Top),
            Rect {
                x: 0,
                y: 216,
                width: 2560,
                height: 1176
            }
        );
        assert_eq!(
            arena(
                area,
                panel_rect(MONITOR, DockEdge::Bottom, 15),
                DockEdge::Bottom
            ),
            Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1224
            }
        );
    }

    #[test]
    fn slots_are_16_by_9_centered_and_inside_the_panel() {
        let panel = panel_rect(MONITOR, DockEdge::Bottom, 15);
        let three = slots(panel, DockEdge::Bottom, 3);
        assert_eq!(three.len(), 3);
        for slot in &three {
            assert_eq!(slot.height, 192);
            assert_eq!(slot.width, 341);
            assert!(slot.y >= panel.y && slot.y + slot.height <= panel.y + panel.height);
        }
        let left = three[0].x - panel.x;
        let right = panel.x + panel.width - (three[2].x + three[2].width);
        assert!(left.abs_diff(right) <= 1);
        assert!(three[1].x >= three[0].x + three[0].width);
    }

    #[test]
    fn many_slots_shrink_to_fit_and_side_panels_stack_vertically() {
        let panel = panel_rect(MONITOR, DockEdge::Bottom, 15);
        let many = slots(panel, DockEdge::Bottom, 12);
        let last = many.last().unwrap();
        assert!(last.x + last.width <= panel.x + panel.width);
        assert!(many[0].width < 341);
        assert_eq!(many[0].width * 9 / 16, many[0].height);

        let side = panel_rect(MONITOR, DockEdge::Left, 15);
        let column = slots(side, DockEdge::Left, 2);
        assert_eq!(column[0].x, column[1].x);
        assert!(column[1].y >= column[0].y + column[0].height);
        assert_eq!(column[0].width * 9 / 16, column[0].height);
    }

    #[test]
    fn the_panel_only_takes_the_length_its_windows_need_from_the_fill_end() {
        let strip = panel_rect(MONITOR, DockEdge::Top, 10);
        // 144 px thick: 120 px slots are 213 px long, with 12 px around each.
        let one = occupied(strip, DockEdge::Top, DockFill::Start, 1);
        assert_eq!(
            one,
            Rect {
                x: 0,
                y: 0,
                width: 237,
                height: 144
            }
        );
        let two = occupied(strip, DockEdge::Top, DockFill::End, 2);
        assert_eq!((two.x, two.width), (2560 - 462, 462));
        // Too many for the edge: the whole strip is used and slots shrink.
        assert_eq!(occupied(strip, DockEdge::Top, DockFill::Start, 20), strip);
        assert_eq!(occupied(strip, DockEdge::Top, DockFill::Start, 0).width, 0);
        // Only a panel that needs the whole edge reserves it.
        assert!(!fills(strip, DockEdge::Top, 1));
        assert!(!fills(strip, DockEdge::Top, 11));
        assert!(fills(strip, DockEdge::Top, 12));

        let side = panel_rect(MONITOR, DockEdge::Right, 10);
        let column = occupied(side, DockEdge::Right, DockFill::End, 1);
        assert_eq!(column.x, side.x);
        assert_eq!(column.y + column.height, MONITOR.height);
        assert!(column.height < side.height);

        // Slots fill exactly, and the first window sits at the fill end.
        let slots = slots_from(two, DockEdge::Top, DockFill::End, 2);
        assert_eq!(slots[0].x + slots[0].width, two.x + two.width - 12);
        assert_eq!(slots[1].x, two.x + 12);
        assert_eq!(slots[0].height, 120);
        assert_eq!(slots_from(one, DockEdge::Top, DockFill::Start, 1)[0].x, 12);
    }

    #[test]
    fn strips_stay_inside_the_work_area_beside_a_taskbar() {
        let work = Rect {
            height: 1392,
            ..MONITOR
        };
        let strip = strip_within(MONITOR, work, DockEdge::Bottom, 10);
        assert_eq!(
            strip,
            Rect {
                x: 0,
                y: 1392 - 144,
                width: 2560,
                height: 144
            }
        );
        assert_eq!(DockFill::Start.label(DockEdge::Top), "Left");
        assert_eq!(DockFill::End.label(DockEdge::Left), "Bottom");
    }

    #[test]
    fn parked_windows_keep_a_covered_corner_on_the_monitor() {
        let size = PREVIEW_SOURCE;
        let park = park_rect(MONITOR, DockEdge::Bottom, DockFill::Start, size, 0, &[]).unwrap();
        assert_eq!((park.x, park.y), (8 - 1920, 1440 - 8));
        assert_eq!((park.width, park.height), PREVIEW_SOURCE);
        let park = park_rect(MONITOR, DockEdge::Right, DockFill::End, size, 0, &[]).unwrap();
        assert_eq!((park.x, park.y), (2560 - 8, 1440 - 8));
        let park = park_rect(MONITOR, DockEdge::Top, DockFill::Start, size, 0, &[]).unwrap();
        assert_eq!((park.x, park.y), (8 - 1920, 8 - 1080));
        // A window stowed at its own size still leaves just the sliver on screen.
        let park = park_rect(MONITOR, DockEdge::Left, DockFill::Start, (800, 600), 0, &[]).unwrap();
        assert_eq!((park.x, park.y), (8 - 800, 8 - 600));
        // A monitor beside that corner would show the window: park elsewhere.
        let left = Rect {
            x: -1920,
            y: 1000,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            park_rect(MONITOR, DockEdge::Bottom, DockFill::Start, size, 0, &[left]),
            None
        );
        assert!(park_rect(MONITOR, DockEdge::Bottom, DockFill::End, size, 0, &[left]).is_some());
    }

    #[test]
    fn blocked_corners_park_in_a_gap_of_the_virtual_screen() {
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        // A 4K primary with monitors offset around it, as on a real desk: the
        // top-left corner touches the upper-left monitor.
        let primary = rect(0, 0, 3840, 2160);
        let monitors = [
            rect(3840, 454, 1920, 1080),
            primary,
            rect(-1920, 913, 1920, 1080),
            rect(-1932, -167, 1920, 1080),
        ];
        let others = [monitors[0], monitors[2], monitors[3]];
        assert_eq!(
            park_rect(
                primary,
                DockEdge::Top,
                DockFill::Start,
                PREVIEW_SOURCE,
                0,
                &others
            ),
            None
        );
        let park = gap_park_rect(&monitors, PREVIEW_SOURCE, 0).unwrap();
        assert_eq!((park.width, park.height), PREVIEW_SOURCE);
        // It touches the box around the monitors, but no monitor shows it.
        assert!(park.y + park.height > -167 && park.x < 5760 && park.x + park.width > -1932);
        assert!(!monitors.iter().any(|m| intersects(park, *m)));
        // Monitors that fill their box leave no gap.
        assert_eq!(gap_park_rect(&[MONITOR], PREVIEW_SOURCE, 0), None);
        let beside = rect(2560, 0, 2560, 1440);
        assert_eq!(gap_park_rect(&[MONITOR, beside], PREVIEW_SOURCE, 0), None);
    }

    #[test]
    fn every_docked_window_keeps_a_corner_no_other_covers() {
        // Chrome stops drawing a window the others cover completely, so each
        // parked window needs a point that none of the others contain.
        let contains = |r: Rect, (x, y): (i32, i32)| {
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        };
        let check = |spots: Vec<Rect>| {
            for (i, spot) in spots.iter().enumerate() {
                let corners = [
                    (spot.x, spot.y),
                    (spot.x + spot.width - 1, spot.y),
                    (spot.x, spot.y + spot.height - 1),
                    (spot.x + spot.width - 1, spot.y + spot.height - 1),
                ];
                assert!(
                    corners.iter().any(|&corner| spots
                        .iter()
                        .enumerate()
                        .all(|(j, other)| j == i || !contains(*other, corner))),
                    "window {i} is covered by the others"
                );
            }
        };
        for (edge, fill) in [
            (DockEdge::Top, DockFill::Start),
            (DockEdge::Bottom, DockFill::End),
            (DockEdge::Right, DockFill::Start),
        ] {
            let spots: Vec<Rect> = (0..6)
                .map(|step| park_rect(MONITOR, edge, fill, PREVIEW_SOURCE, step, &[]).unwrap())
                .collect();
            // Each step moves the window further under the panel's corner.
            assert_ne!(spots[0], spots[1]);
            check(spots);
        }
        let beside = Rect {
            x: 2560,
            y: 400,
            width: 1920,
            height: 1080,
        };
        let monitors = [MONITOR, beside];
        let spots: Vec<Rect> = (0..6)
            .map(|step| gap_park_rect(&monitors, PREVIEW_SOURCE, step).unwrap())
            .collect();
        for spot in &spots {
            assert!(!monitors.iter().any(|m| intersects(*spot, *m)));
        }
        check(spots);
    }

    #[test]
    fn windows_grow_to_1080p_where_all_of_them_is_on_screen() {
        let current = Rect {
            x: 2000,
            y: 900,
            width: 800,
            height: 600,
        };
        // Pulled back so the whole 1920x1080 fits on the 2560x1440 monitor.
        assert_eq!(
            staging_rect(current, MONITOR),
            Rect {
                x: 640,
                y: 360,
                width: 1920,
                height: 1080
            }
        );
        let near_origin = Rect {
            x: 100,
            y: 50,
            ..current
        };
        assert_eq!(
            (
                staging_rect(near_origin, MONITOR).x,
                staging_rect(near_origin, MONITOR).y
            ),
            (100, 50)
        );
        // A window off to the left still stages on the monitor.
        let off = Rect { x: -900, ..current };
        assert_eq!(staging_rect(off, MONITOR).x, 0);
    }

    #[test]
    fn windows_the_panel_covers_stop_short_of_it_and_others_are_untouched() {
        // A one-window panel at the top of the right edge.
        let strip = panel_rect(MONITOR, DockEdge::Right, 10);
        let panel = occupied(strip, DockEdge::Right, DockFill::Start, 1);
        let right_half = Rect {
            x: 1280,
            y: 0,
            width: 1280,
            height: 1440,
        };
        assert_eq!(
            avoid(right_half, panel, DockEdge::Right, 20),
            Rect {
                width: panel.x - 20 - 1280,
                ..right_half
            }
        );
        let bottom_right = Rect {
            x: 1280,
            y: 720,
            width: 1280,
            height: 720,
        };
        assert_eq!(
            avoid(bottom_right, panel, DockEdge::Right, 20),
            bottom_right
        );
        let bottom = panel_rect(MONITOR, DockEdge::Bottom, 10);
        let full = Rect {
            x: 0,
            y: 0,
            ..MONITOR
        };
        let squeezed = avoid(full, bottom, DockEdge::Bottom, 0);
        assert_eq!(squeezed.y + squeezed.height, bottom.y);
        let left = panel_rect(MONITOR, DockEdge::Left, 10);
        assert_eq!(avoid(full, left, DockEdge::Left, 5).x, left.width + 5);
        let top = panel_rect(MONITOR, DockEdge::Top, 10);
        assert_eq!(avoid(full, top, DockEdge::Top, 0).y, top.height);
    }

    #[test]
    fn previews_crop_the_middle_of_the_window_to_the_slot_shape() {
        let wide = Rect {
            x: 7,
            y: 0,
            width: 1920,
            height: 960,
        };
        let cropped = crop_to_aspect(wide, 16, 9);
        assert_eq!((cropped.width, cropped.height), (1706, 960));
        assert_eq!(cropped.x, 7 + (1920 - 1706) / 2);
        let tall = Rect {
            x: 0,
            y: 0,
            width: 900,
            height: 1200,
        };
        let cropped = crop_to_aspect(tall, 16, 9);
        assert_eq!((cropped.width, cropped.height), (900, 506));
        assert_eq!(cropped.y, (1200 - 506) / 2);
        let exact = Rect {
            width: 1920,
            height: 1080,
            ..tall
        };
        assert_eq!(crop_to_aspect(exact, 384, 216), exact);
    }

    #[test]
    fn the_area_shrinks_to_the_previews_on_its_inner_side() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 384,
            height: 216,
        };
        // A 2:1 window's preview, pushed against the top edge.
        let wide = Rect {
            x: 3,
            y: 3,
            width: 378,
            height: 189,
        };
        assert_eq!(shrink_to(area, DockEdge::Top, &[wide], 3).height, 195);
        let bottom = Rect { y: 1944, ..area };
        let shown = Rect {
            y: 1944 + 24,
            ..wide
        };
        let shrunk = shrink_to(bottom, DockEdge::Bottom, &[shown], 3);
        assert_eq!((shrunk.y, shrunk.y + shrunk.height), (1944 + 21, 1944 + 216));
        let side = Rect {
            x: 3456,
            ..area
        };
        let tall = Rect {
            x: 3456 + 100,
            ..wide
        };
        let right = shrink_to(side, DockEdge::Right, &[tall], 3);
        assert_eq!((right.x, right.x + right.width), (3456 + 97, 3456 + 384));
        assert_eq!(shrink_to(area, DockEdge::Left, &[], 3), area);
    }

    #[test]
    fn snaps_leave_the_same_spacing_beside_the_panel_as_between_windows() {
        // Two windows inset by 8 each sit 16 apart; the panel's previews
        // already sit 3 inside its edge.
        assert_eq!(snap_gap(8) + SLOT_PADDING, 16);
        assert_eq!(snap_gap(0), 0);
        assert_eq!(snap_gap(1), 0);
    }

    #[test]
    fn hit_testing_finds_the_slot_under_the_cursor() {
        let panel = panel_rect(MONITOR, DockEdge::Bottom, 15);
        let two = slots(panel, DockEdge::Bottom, 2);
        let inside = (two[1].x + 5, two[1].y + 5);
        assert_eq!(hit_test(&two, inside), Some(1));
        assert_eq!(hit_test(&two, (panel.x + 1, panel.y + 1)), None);
        assert!(slots(panel, DockEdge::Bottom, 0).is_empty());
    }
}
