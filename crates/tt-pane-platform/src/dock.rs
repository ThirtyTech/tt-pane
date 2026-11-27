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

/// Where to park a docked window: nearly off the monitor past the panel's fill
/// corner, with a `PARK_SLIVER` square left on screen under the panel (or the
/// taskbar). Applications such as Explorer stop drawing a window that is on no
/// monitor at all, which freezes its preview. `None` when that spot would show
/// part of the window on one of the `others` monitors.
pub fn park_rect(monitor: Rect, edge: DockEdge, fill: DockFill, others: &[Rect]) -> Option<Rect> {
    let (w, h) = PREVIEW_SOURCE;
    // The monitor corner at the fill end of the edge.
    let (right, bottom) = match (edge, fill) {
        (DockEdge::Top, DockFill::Start) | (DockEdge::Left, DockFill::Start) => (false, false),
        (DockEdge::Top, DockFill::End) | (DockEdge::Right, DockFill::Start) => (true, false),
        (DockEdge::Bottom, DockFill::Start) | (DockEdge::Left, DockFill::End) => (false, true),
        (DockEdge::Bottom, DockFill::End) | (DockEdge::Right, DockFill::End) => (true, true),
    };
    let rect = Rect {
        x: if right {
            monitor.x + monitor.width - PARK_SLIVER
        } else {
            monitor.x - w + PARK_SLIVER
        },
        y: if bottom {
            monitor.y + monitor.height - PARK_SLIVER
        } else {
            monitor.y - h + PARK_SLIVER
        },
        width: w,
        height: h,
    };
    let overlaps = |o: &Rect| {
        rect.x < o.x + o.width
            && o.x < rect.x + rect.width
            && rect.y < o.y + o.height
            && o.y < rect.y + rect.height
    };
    (!others.iter().any(overlaps)).then_some(rect)
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
        let park = park_rect(MONITOR, DockEdge::Bottom, DockFill::Start, &[]).unwrap();
        assert_eq!((park.x, park.y), (8 - 1920, 1440 - 8));
        assert_eq!((park.width, park.height), PREVIEW_SOURCE);
        let park = park_rect(MONITOR, DockEdge::Right, DockFill::End, &[]).unwrap();
        assert_eq!((park.x, park.y), (2560 - 8, 1440 - 8));
        let park = park_rect(MONITOR, DockEdge::Top, DockFill::Start, &[]).unwrap();
        assert_eq!((park.x, park.y), (8 - 1920, 8 - 1080));
        // A monitor beside that corner would show the window: park elsewhere.
        let left = Rect {
            x: -1920,
            y: 1000,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            park_rect(MONITOR, DockEdge::Bottom, DockFill::Start, &[left]),
            None
        );
        assert!(park_rect(MONITOR, DockEdge::Bottom, DockFill::End, &[left]).is_some());
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
