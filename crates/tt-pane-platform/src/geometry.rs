#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Desktop coordinates, in the native backend's units. Origins may be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Insets in native desktop units, matching Hyprland's per-window gap defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnapGaps {
    pub inner: u32,
    pub outer: u32,
}

impl Default for SnapGaps {
    fn default() -> Self {
        Self {
            inner: 5,
            outer: 20,
        }
    }
}

impl SnapGaps {
    pub fn inset(self, area: Rect, target: Rect) -> Rect {
        let left = if target.x == area.x {
            self.outer
        } else {
            self.inner
        };
        let top = if target.y == area.y {
            self.outer
        } else {
            self.inner
        };
        let right = if i64::from(target.x) + i64::from(target.width)
            == i64::from(area.x) + i64::from(area.width)
        {
            self.outer
        } else {
            self.inner
        };
        let bottom = if i64::from(target.y) + i64::from(target.height)
            == i64::from(area.y) + i64::from(area.height)
        {
            self.outer
        } else {
            self.inner
        };
        inset_rect(target, left, top, right, bottom)
    }

    pub fn work_area(self, area: Rect) -> Rect {
        self.inset(area, area)
    }
}

/// Clamp insets so even very small work areas retain a positive rectangle.
pub fn inset_rect(rect: Rect, left: u32, top: u32, right: u32, bottom: u32) -> Rect {
    let left = left.min((rect.width.max(1) - 1) as u32 / 2) as i32;
    let right = right.min((rect.width.max(1) - 1) as u32 / 2) as i32;
    let top = top.min((rect.height.max(1) - 1) as u32 / 2) as i32;
    let bottom = bottom.min((rect.height.max(1) - 1) as u32 / 2) as i32;
    Rect {
        x: rect.x + left,
        y: rect.y + top,
        width: rect.width - left - right,
        height: rect.height - top - bottom,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HorizontalAnchor {
    Left,
    Center,
    Right,
}

impl HorizontalAnchor {
    /// Keep the edge on the window's side of the monitor, or its center when centered.
    pub fn for_window(area: Rect, current: Rect) -> Self {
        let offset = 2 * i64::from(current.x) + i64::from(current.width)
            - 2 * i64::from(area.x)
            - i64::from(area.width);
        if offset.abs() <= 2 {
            Self::Center
        } else if offset < 0 {
            Self::Left
        } else {
            Self::Right
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
    Maximize,
    ShrinkWidth,
    ExpandWidth,
    FillGap,
    ResizeUp,
    ResizeDown,
    FocusLeft,
    FocusDown,
    FocusUp,
    FocusRight,
}

impl Action {
    pub const POSITIONS: [Self; 10] = [
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Center,
        Self::Maximize,
    ];

    pub const ALL: [Self; 19] = [
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Center,
        Self::Maximize,
        Self::ShrinkWidth,
        Self::ExpandWidth,
        Self::FillGap,
        Self::ResizeUp,
        Self::ResizeDown,
        Self::FocusLeft,
        Self::FocusDown,
        Self::FocusUp,
        Self::FocusRight,
    ];

    pub fn focuses(self) -> bool {
        matches!(
            self,
            Self::FocusLeft | Self::FocusDown | Self::FocusUp | Self::FocusRight
        )
    }

    pub fn adjusts_pair(self) -> bool {
        matches!(self, Self::ResizeUp | Self::ResizeDown)
    }

    pub fn adjusts_width(self) -> bool {
        matches!(self, Self::ShrinkWidth | Self::ExpandWidth)
    }

    pub fn horizontal_anchor(self) -> Option<HorizontalAnchor> {
        match self {
            Self::Left | Self::TopLeft | Self::BottomLeft => Some(HorizontalAnchor::Left),
            Self::Right | Self::TopRight | Self::BottomRight => Some(HorizontalAnchor::Right),
            Self::Top | Self::Bottom | Self::Center | Self::Maximize => {
                Some(HorizontalAnchor::Center)
            }
            Self::ShrinkWidth
            | Self::ExpandWidth
            | Self::FillGap
            | Self::ResizeUp
            | Self::ResizeDown
            | Self::FocusLeft
            | Self::FocusDown
            | Self::FocusUp
            | Self::FocusRight => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::Center => "center",
            Self::Maximize => "maximize",
            Self::ShrinkWidth => "shrink-width",
            Self::ExpandWidth => "expand-width",
            Self::FillGap => "fill-gap",
            Self::ResizeUp => "resize-up",
            Self::FocusLeft => "focus-left",
            Self::FocusDown => "focus-down",
            Self::FocusUp => "focus-up",
            Self::FocusRight => "focus-right",
            Self::ResizeDown => "resize-down",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left half",
            Self::Right => "Right half",
            Self::Top => "Top half",
            Self::Bottom => "Bottom half",
            Self::TopLeft => "Top left quarter",
            Self::TopRight => "Top right quarter",
            Self::BottomLeft => "Bottom left quarter",
            Self::BottomRight => "Bottom right quarter",
            Self::Center => "Center window",
            Self::Maximize => "Fill work area",
            Self::ShrinkWidth => "Shrink width",
            Self::ExpandWidth => "Expand width",
            Self::FillGap => "Fill last gap",
            Self::ResizeUp => "Move shared boundary up",
            Self::FocusLeft => "Focus left",
            Self::FocusDown => "Focus down",
            Self::FocusUp => "Focus up",
            Self::FocusRight => "Focus right",
            Self::ResizeDown => "Move shared boundary down",
        }
    }

    pub fn target(self, area: Rect, current: Rect) -> Rect {
        let half_w = area.width / 2;
        let half_h = area.height / 2;
        let mut target = area;
        match self {
            // Contextual action: the coordinator resolves its saved rectangle.
            Self::FillGap
            | Self::ResizeUp
            | Self::ResizeDown
            | Self::FocusLeft
            | Self::FocusDown
            | Self::FocusUp
            | Self::FocusRight => return current,
            Self::ShrinkWidth | Self::ExpandWidth => {
                return current.adjust_width(
                    area,
                    HorizontalAnchor::for_window(area, current),
                    if self == Self::ShrinkWidth { -25 } else { 25 },
                );
            }
            Self::Center => {
                target.width = half_w;
                target.height = half_h;
                target.x += (area.width - target.width) / 2;
                target.y += (area.height - target.height) / 2;
            }
            Self::Maximize => {}
            Self::Left | Self::Right => {
                target.width = if self == Self::Left {
                    half_w
                } else {
                    area.width - half_w
                };
                if self == Self::Right {
                    target.x += half_w;
                }
            }
            Self::Top | Self::Bottom => {
                target.height = if self == Self::Top {
                    half_h
                } else {
                    area.height - half_h
                };
                if self == Self::Bottom {
                    target.y += half_h;
                }
            }
            _ => {
                let right = matches!(self, Self::TopRight | Self::BottomRight);
                let bottom = matches!(self, Self::BottomLeft | Self::BottomRight);
                target.width = if right { area.width - half_w } else { half_w };
                target.height = if bottom { area.height - half_h } else { half_h };
                if right {
                    target.x += half_w;
                }
                if bottom {
                    target.y += half_h;
                }
            }
        }
        target
    }

    /// Scale the original placement while keeping its corner/edge/center anchored.
    /// Dimensions are independently capped at the current monitor's work area.
    pub fn grown_target(self, area: Rect, base: Rect, multiplier: f64) -> Rect {
        // Avoid rounding exact half-pixels down due to accumulated binary float error.
        let width = ((f64::from(base.width) * multiplier + 1e-9).round() as i32)
            .clamp(1, area.width.max(1));
        let height = ((f64::from(base.height) * multiplier + 1e-9).round() as i32)
            .clamp(1, area.height.max(1));
        let x = match self {
            Self::Left | Self::TopLeft | Self::BottomLeft => area.x,
            Self::Right | Self::TopRight | Self::BottomRight => area.x + area.width - width,
            _ => area.x + (area.width - width) / 2,
        };
        let y = match self {
            Self::Top | Self::TopLeft | Self::TopRight => area.y,
            Self::Bottom | Self::BottomLeft | Self::BottomRight => area.y + area.height - height,
            _ => area.y + (area.height - height) / 2,
        };
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

/// Resize a vertically adjacent pair with matching horizontal edges. Preserve the
/// outside edges and the existing gap. The step is 25% of the selected window's current height.
/// Approaching the midpoint snaps to equal heights within one standard half-height
/// step, or whenever the ordinary step would cross it. Odd totals give bottom the extra pixel.
/// Returns bounds in the same order as the inputs.
pub fn resize_vertical_pair(selected: Rect, neighbor: Rect, up: bool) -> Option<(Rect, Rect)> {
    if selected.width <= 0
        || neighbor.width <= 0
        || selected.height <= 0
        || neighbor.height <= 0
        || selected.x.abs_diff(neighbor.x) > 2
        || (i64::from(selected.x) + i64::from(selected.width)
            - i64::from(neighbor.x)
            - i64::from(neighbor.width))
        .abs()
            > 2
    {
        return None;
    }
    let selected_above = selected.y < neighbor.y;
    let (top, bottom) = if selected_above {
        (selected, neighbor)
    } else {
        (neighbor, selected)
    };
    let gap = i64::from(bottom.y) - i64::from(top.y) - i64::from(top.height);
    if !(-2..=i64::from(crate::SNAP_GAPS.get().inner * 2).max(16)).contains(&gap) {
        return None;
    }
    let step = ((i64::from(selected.height) + 2) / 4).max(1);
    let midpoint = (i64::from(top.height) + i64::from(bottom.height)) / 2;
    let to_midpoint = midpoint - i64::from(top.height);
    let midpoint_step = ((midpoint + 2) / 4).max(1);
    let mut delta = if up { -step } else { step };
    // Include the return from a smaller selected window: its current 25% step
    // can fall short of the midpoint after the initial 25% shrink.
    if to_midpoint.signum() == delta.signum() && to_midpoint.abs() <= step.max(midpoint_step) {
        delta = to_midpoint;
    }
    let delta = delta.clamp(1 - i64::from(top.height), i64::from(bottom.height) - 1);
    let top = Rect {
        height: i32::try_from(i64::from(top.height) + delta).ok()?,
        ..top
    };
    let bottom = Rect {
        y: i32::try_from(i64::from(bottom.y) + delta).ok()?,
        height: i32::try_from(i64::from(bottom.height) - delta).ok()?,
        ..bottom
    };
    Some(if selected_above {
        (top, bottom)
    } else {
        (bottom, top)
    })
}

impl Rect {
    /// The largest full-height strip freed by a width reduction; ties prefer right.
    pub fn freed_width_gap(self, reduced: Self) -> Option<Self> {
        if reduced.width <= 0
            || reduced.width >= self.width
            || reduced.y != self.y
            || reduced.height != self.height
            || reduced.x < self.x
            || i64::from(reduced.x) + i64::from(reduced.width)
                > i64::from(self.x) + i64::from(self.width)
        {
            return None;
        }
        let left = reduced.x - self.x;
        let right = self.width - left - reduced.width;
        if right >= left && right > 0 {
            Some(Self {
                x: reduced.x + reduced.width,
                width: right,
                ..self
            })
        } else if left > 0 {
            Some(Self {
                width: left,
                ..self
            })
        } else {
            None
        }
    }
    /// Preserve a width-only request's anchor when an application enforces size limits.
    pub fn align_width_result(self, requested: Self, actual: Self) -> Self {
        if requested.y != self.y
            || requested.height != self.height
            || requested.width == self.width
            || actual.width == requested.width
            || requested.x == self.x
        {
            return actual;
        }
        let right = i64::from(self.x) + i64::from(self.width);
        let x = if i64::from(requested.x) + i64::from(requested.width) == right {
            right - i64::from(actual.width)
        } else if (2 * i64::from(requested.x) + i64::from(requested.width)
            - 2 * i64::from(self.x)
            - i64::from(self.width))
        .abs()
            <= 1
        {
            i64::from(self.x) + (i64::from(self.width) - i64::from(actual.width)).div_euclid(2)
        } else {
            return actual;
        };
        Self {
            x: x as i32,
            ..actual
        }
    }

    /// Adjust only width, preserving the chosen edge or midpoint and all vertical geometry.
    pub fn adjust_width(self, area: Rect, anchor: HorizontalAnchor, percent: i32) -> Self {
        self.adjust_width_from(area, anchor, percent, self)
    }

    /// Reuse the initial rectangle's anchor to avoid half-pixel drift over many presses.
    pub fn adjust_width_from(
        self,
        area: Rect,
        anchor: HorizontalAnchor,
        percent: i32,
        reference: Rect,
    ) -> Self {
        let current_width = i64::from(self.width.max(1));
        let width = ((current_width * i64::from(100 + percent.clamp(-99, 100)) + 50) / 100).max(1);
        let width = if percent < 0 {
            width.min((current_width - 1).max(1))
        } else if percent > 0 {
            width.max(current_width + 1)
        } else {
            width
        };
        let left = i64::from(reference.x);
        let right = left + i64::from(reference.width);
        let area_left = i64::from(area.x);
        let area_right = area_left + i64::from(area.width);
        let max_width = match anchor {
            HorizontalAnchor::Left => area_right - left,
            HorizontalAnchor::Right => right - area_left,
            HorizontalAnchor::Center => {
                (left + right - 2 * area_left).min(2 * area_right - left - right)
            }
        }
        .clamp(1, i64::from(i32::MAX));
        // Expanding an already oversized/off-screen window must not shrink it.
        let width = if percent > 0 {
            width.min(max_width).max(current_width)
        } else {
            width
        };
        let width = width.min(i64::from(i32::MAX)) as i32;
        let x = match anchor {
            HorizontalAnchor::Left => reference.x,
            HorizontalAnchor::Right => (right - i64::from(width)) as i32,
            HorizontalAnchor::Center => {
                (left + (i64::from(reference.width) - i64::from(width)).div_euclid(2)) as i32
            }
        };
        Self { x, width, ..self }
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let width = (self.x + self.width).min(other.x + other.width) - x;
        let height = (self.y + self.height).min(other.y + other.height) - y;
        (width > 0 && height > 0).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// Largest overlap wins; windows completely off-screen use the nearest monitor.
pub fn monitor_for(window: Rect, monitors: &[Rect]) -> Option<usize> {
    monitors
        .iter()
        .enumerate()
        .max_by_key(|(_, monitor)| {
            let overlap = window
                .intersection(**monitor)
                .map_or(0, |r| i64::from(r.width) * i64::from(r.height));
            let cx = i64::from(window.x) * 2 + i64::from(window.width);
            let cy = i64::from(window.y) * 2 + i64::from(window.height);
            let mx = i64::from(monitor.x) * 2 + i64::from(monitor.width);
            let my = i64::from(monitor.y) * 2 + i64::from(monitor.height);
            (overlap, -((cx - mx).pow(2) + (cy - my).pow(2)))
        })
        .map(|(index, _)| index)
}

/// Leave a centered group spatially; vertical exits prefer the leftmost column.
/// Candidates have already been filtered to non-members on the same monitor.
pub fn focus_group_exit(current: Rect, candidates: &[Rect], action: Action) -> Option<usize> {
    if matches!(action, Action::FocusLeft | Action::FocusRight) {
        return focus_neighbor(current, candidates, action);
    }
    let cy = 2 * i64::from(current.y) + i64::from(current.height);
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, rect)| {
            if rect.width <= 0 || rect.height <= 0 {
                return None;
            }
            let dy = 2 * i64::from(rect.y) + i64::from(rect.height) - cy;
            let forward = match action {
                Action::FocusUp => -dy,
                Action::FocusDown => dy,
                _ => return None,
            };
            (forward > 0).then_some((index, rect.x, forward))
        })
        .min_by_key(|&(index, x, forward)| (x, forward, index))
        .map(|(index, _, _)| index)
}

/// Rank windows by center distance in a 90-degree directional cone. This permits
/// diagonal steps through a center window without jumping past it to a far edge.
/// Equal scores retain native Z-order (the topmost candidate wins).
pub fn focus_neighbor(current: Rect, candidates: &[Rect], action: Action) -> Option<usize> {
    if !action.focuses() || current.width <= 0 || current.height <= 0 {
        return None;
    }
    let center = |r: Rect| {
        (
            2 * i64::from(r.x) + i64::from(r.width),
            2 * i64::from(r.y) + i64::from(r.height),
        )
    };
    let (cx, cy) = center(current);
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, &rect)| {
            if rect.width <= 0 || rect.height <= 0 {
                return None;
            }
            let (x, y) = center(rect);
            let (dx, dy) = (x - cx, y - cy);
            let (forward, sideways) = match action {
                Action::FocusLeft => (-dx, dy),
                Action::FocusRight => (dx, dy),
                Action::FocusUp => (-dy, dx),
                Action::FocusDown => (dy, dx),
                _ => return None,
            };
            if forward <= 0 || sideways.abs() > forward {
                return None;
            }
            let distance = i128::from(dx).pow(2) + i128::from(dy).pow(2);
            Some((index, distance))
        })
        .min_by_key(|&(index, distance)| (distance, index))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centered_vertical_exit_reaches_wide_screen_corners_left_first() {
        let area = Rect {
            x: -3440,
            y: 40,
            width: 3440,
            height: 1440,
        };
        let center = Action::Center.target(area, area);
        let corners = [
            Action::TopRight,
            Action::BottomRight,
            Action::TopLeft,
            Action::BottomLeft,
        ]
        .map(|action| action.target(area, area));
        assert_eq!(focus_neighbor(center, &corners, Action::FocusDown), None);
        assert_eq!(focus_neighbor(center, &corners, Action::FocusUp), None);
        assert_eq!(
            focus_group_exit(center, &corners, Action::FocusDown),
            Some(3)
        );
        assert_eq!(focus_group_exit(center, &corners, Action::FocusUp), Some(2));
        assert_eq!(
            focus_group_exit(center, &corners[..2], Action::FocusDown),
            Some(1)
        );
        assert_eq!(
            focus_group_exit(center, &corners[..1], Action::FocusDown),
            None
        );
        assert_eq!(focus_group_exit(center, &[], Action::FocusUp), None);
        assert_eq!(
            focus_group_exit(center, &corners, Action::FocusLeft),
            Some(2)
        );
        assert_eq!(
            focus_group_exit(center, &corners, Action::FocusRight),
            Some(0)
        );
        assert_eq!(
            focus_group_exit(center, &corners[..2], Action::FocusLeft),
            None
        );
    }
    #[test]
    fn freed_gap_covers_only_the_reduced_part_of_the_original_position() {
        let area = Rect {
            x: -2000,
            y: 40,
            width: 2000,
            height: 1200,
        };
        for percent in [25, 50] {
            let left = Action::BottomLeft.target(area, area);
            let reduced = left.adjust_width(area, HorizontalAnchor::Left, -percent);
            let gap = left.freed_width_gap(reduced).unwrap();
            assert_eq!(
                gap,
                Rect {
                    x: reduced.x + reduced.width,
                    width: 1000 - reduced.width,
                    ..left
                }
            );
            assert_eq!(gap.x + gap.width, left.x + left.width);
            assert_eq!(gap.intersection(reduced), None);
            let right = Action::BottomRight.target(area, area);
            let reduced = right.adjust_width(area, HorizontalAnchor::Right, -percent);
            assert_eq!(
                right.freed_width_gap(reduced).unwrap(),
                Rect {
                    width: 1000 - reduced.width,
                    ..right
                }
            );
        }
        let base = Action::Center.target(area, area);
        let reduced = base.adjust_width(area, HorizontalAnchor::Center, -50);
        assert_eq!(
            base.freed_width_gap(reduced),
            Some(Rect {
                x: -750,
                y: 340,
                width: 250,
                height: 600
            })
        );
        assert_eq!(base.freed_width_gap(base), None);
        assert_eq!(
            base.freed_width_gap(Rect {
                height: 400,
                ..reduced
            }),
            None
        );
    }
    #[test]
    fn paired_resize_uses_selected_height_and_preserves_outer_edges() {
        for (y, height, gap) in [(-1000, 500, 0), (40, 501, 8)] {
            let top = Rect {
                x: -1920,
                y,
                width: 960,
                height,
            };
            let bottom = Rect {
                y: y + height + gap,
                ..top
            };
            let step = (height + 2) / 4;
            for selected_top in [false, true] {
                let (selected, neighbor) = if selected_top {
                    (top, bottom)
                } else {
                    (bottom, top)
                };
                let (a, b) = resize_vertical_pair(selected, neighbor, true).unwrap();
                let (new_top, new_bottom) = if selected_top { (a, b) } else { (b, a) };
                assert_eq!(new_top.height, height - step);
                assert_eq!(new_bottom.height, height + step);
                assert_eq!(new_top.y, top.y);
                assert_eq!(new_bottom.y + new_bottom.height, bottom.y + bottom.height);
                assert_eq!(new_bottom.y - new_top.y - new_top.height, gap);
                assert_eq!(
                    resize_vertical_pair(a, b, false),
                    Some((selected, neighbor))
                );
            }
        }
    }

    #[test]
    fn paired_resize_step_follows_the_selected_window_after_each_press() {
        let top = Rect {
            x: 0,
            y: 0,
            width: 500,
            height: 400,
        };
        let bottom = Rect { y: 400, ..top };
        let (bottom, top) = resize_vertical_pair(bottom, top, true).unwrap();
        assert_eq!((bottom.height, top.height), (500, 300)); // 62.5 / 37.5
        let (bottom, top) = resize_vertical_pair(bottom, top, true).unwrap();
        assert_eq!((bottom.height, top.height), (625, 175)); // Another 25% of selected height
        let (bottom, top) = resize_vertical_pair(bottom, top, false).unwrap();
        assert_eq!((bottom.height, top.height), (469, 331)); // Still outside midpoint hop
        let (bottom, top) = resize_vertical_pair(bottom, top, false).unwrap();
        assert_eq!((bottom.height, top.height), (400, 400)); // Stop at equal before crossing
        let (bottom, top) = resize_vertical_pair(bottom, top, false).unwrap();
        assert_eq!((bottom.height, top.height), (300, 500)); // Can continue past equal
        let (bottom, top) = resize_vertical_pair(bottom, top, true).unwrap();
        assert_eq!((bottom.height, top.height), (400, 400)); // Return from smaller side too
    }

    #[test]
    fn equal_split_is_reachable_from_both_directions_and_either_selected_window() {
        for total in [800, 801] {
            let top = Rect {
                x: -1000,
                y: -600,
                width: 400,
                height: total / 2,
            };
            let bottom = Rect {
                y: top.y + top.height + 8,
                height: total - top.height,
                ..top
            };
            for select_top in [false, true] {
                let original = if select_top {
                    (top, bottom)
                } else {
                    (bottom, top)
                };
                for up in [false, true] {
                    let changed = resize_vertical_pair(original.0, original.1, up).unwrap();
                    assert_ne!(changed, original);
                    assert_eq!(
                        resize_vertical_pair(changed.0, changed.1, !up),
                        Some(original)
                    );
                }
            }
        }
    }

    #[test]
    fn paired_resize_rejects_non_neighbors_and_never_collapses() {
        let top = Rect {
            x: 0,
            y: 0,
            width: 500,
            height: 500,
        };
        let bottom = Rect { y: 500, ..top };
        for invalid in [
            Rect { x: 500, ..bottom },
            Rect {
                width: 250,
                ..bottom
            },
            Rect { y: 600, ..bottom },
            Rect { y: 400, ..bottom },
            Rect {
                height: 0,
                ..bottom
            },
        ] {
            assert!(resize_vertical_pair(top, invalid, true).is_none());
        }
        let mut pair = (top, bottom);
        for _ in 0..40 {
            pair = resize_vertical_pair(pair.0, pair.1, true).unwrap();
        }
        assert_eq!(pair.0.height, 1);
        assert_eq!(pair.1.height, 999);
        assert_eq!(pair.1.y, 1);
    }

    const LEFT: Rect = Rect {
        x: -1920,
        y: 80,
        width: 1920,
        height: 1040,
    };
    #[test]
    fn width_adjustments_preserve_vertical_bounds_and_anchor() {
        for (action, expected_x) in [
            (Action::BottomLeft, -1920),
            (Action::BottomRight, -720),
            (Action::Center, -1320),
        ] {
            let base = action.target(LEFT, LEFT);
            let anchor = action.horizontal_anchor().unwrap();
            let smaller = base.adjust_width(LEFT, anchor, -25);
            assert_eq!(
                smaller,
                Rect {
                    x: expected_x,
                    width: 720,
                    ..base
                }
            );
            let larger = base.adjust_width(LEFT, anchor, 25);
            assert_eq!(larger.width, 1200);
            assert_eq!((larger.y, larger.height), (base.y, base.height));
            let max = base.adjust_width(LEFT, anchor, 100);
            assert_eq!((max.x, max.width), (LEFT.x, LEFT.width));
            assert_eq!(max.adjust_width(LEFT, anchor, 25), max);
        }
    }

    #[test]
    fn offcenter_expansion_stops_at_nearest_available_boundary() {
        let current = Rect {
            x: -1700,
            y: 320,
            width: 800,
            height: 500,
        };
        for (anchor, expected_x, expected_width) in [
            (HorizontalAnchor::Left, -1700, 1700),
            (HorizontalAnchor::Right, -1920, 1020),
            (HorizontalAnchor::Center, -1920, 1240),
        ] {
            let mut window = current;
            for _ in 0..20 {
                window = window.adjust_width_from(LEFT, anchor, 25, current);
            }
            assert_eq!(
                window,
                Rect {
                    x: expected_x,
                    width: expected_width,
                    ..current
                }
            );
        }
        assert_eq!(
            HorizontalAnchor::for_window(LEFT, current),
            HorizontalAnchor::Left
        );
        assert_eq!(
            HorizontalAnchor::for_window(LEFT, Action::Right.target(LEFT, LEFT)),
            HorizontalAnchor::Right
        );
        assert_eq!(
            HorizontalAnchor::for_window(LEFT, Action::Center.target(LEFT, LEFT)),
            HorizontalAnchor::Center
        );
    }

    #[test]
    fn width_minimum_and_rounding_do_not_drift_the_anchor() {
        let base = Rect {
            x: -1441,
            y: 51,
            width: 961,
            height: 703,
        };
        let mut current = base;
        for _ in 0..80 {
            current = current.adjust_width_from(LEFT, HorizontalAnchor::Center, -25, base);
            assert!(current.width >= 1);
            assert!((2 * current.x + current.width - 2 * base.x - base.width).abs() <= 1);
            assert_eq!((current.y, current.height), (base.y, base.height));
        }
        assert_eq!(current.width, 1);
        for anchor in [HorizontalAnchor::Right, HorizontalAnchor::Center] {
            let requested = base.adjust_width(LEFT, anchor, -25);
            let actual = Rect {
                width: 850,
                ..requested
            };
            let aligned = base.align_width_result(requested, actual);
            if anchor == HorizontalAnchor::Right {
                assert_eq!(aligned.x + aligned.width, base.x + base.width);
            } else {
                assert!((2 * aligned.x + aligned.width - 2 * base.x - base.width).abs() <= 1);
            }
        }
    }
    #[test]
    fn growth_preserves_each_anchor_on_negative_origin_monitor() {
        let area = Rect {
            x: -2000,
            y: -1000,
            width: 2000,
            height: 1000,
        };
        for (action, x, y) in [
            (Action::TopLeft, -2000, -1000),
            (Action::TopRight, -1150, -1000),
            (Action::BottomLeft, -2000, -575),
            (Action::BottomRight, -1150, -575),
        ] {
            assert_eq!(
                action.grown_target(area, action.target(area, area), 1.15),
                Rect {
                    x,
                    y,
                    width: 1150,
                    height: 575
                }
            );
        }
        for (action, expected) in [
            (
                Action::Left,
                Rect {
                    x: -2000,
                    y: -1000,
                    width: 1150,
                    height: 1000,
                },
            ),
            (
                Action::Right,
                Rect {
                    x: -1150,
                    y: -1000,
                    width: 1150,
                    height: 1000,
                },
            ),
            (
                Action::Top,
                Rect {
                    x: -2000,
                    y: -1000,
                    width: 2000,
                    height: 575,
                },
            ),
            (
                Action::Bottom,
                Rect {
                    x: -2000,
                    y: -575,
                    width: 2000,
                    height: 575,
                },
            ),
        ] {
            assert_eq!(
                action.grown_target(area, action.target(area, area), 1.15),
                expected
            );
        }
        let current = Rect {
            width: 800,
            height: 400,
            ..area
        };
        let base = Action::Center.target(area, current);
        assert_eq!(
            Action::Center.grown_target(area, base, 1.15),
            Rect {
                x: -1575,
                y: -788,
                width: 1150,
                height: 575
            }
        );
        assert_eq!(Action::Center.grown_target(area, base, 100.0), area);
    }
    #[test]
    fn quarters_are_relative_to_secondary_work_area() {
        assert_eq!(
            Action::TopLeft.target(LEFT, LEFT),
            Rect {
                x: -1920,
                y: 80,
                width: 960,
                height: 520
            }
        );
        assert_eq!(
            Action::BottomRight.target(LEFT, LEFT),
            Rect {
                x: -960,
                y: 600,
                width: 960,
                height: 520
            }
        );
    }
    #[test]
    fn odd_dimensions_have_no_missing_pixel() {
        let area = Rect {
            x: 0,
            y: -1001,
            width: 1921,
            height: 1001,
        };
        let left = Action::Left.target(area, area);
        let right = Action::Right.target(area, area);
        assert_eq!(left.width + right.width, area.width);
        assert_eq!(left.x + left.width, right.x);
    }
    #[test]
    fn monitor_uses_overlap_and_offscreen_fallback() {
        let right = Rect { x: 0, ..LEFT };
        assert_eq!(
            monitor_for(
                Rect {
                    x: -1000,
                    y: 100,
                    width: 1200,
                    height: 500
                },
                &[LEFT, right]
            ),
            Some(0)
        );
        assert_eq!(
            monitor_for(
                Rect {
                    x: 4000,
                    y: 0,
                    width: 500,
                    height: 500
                },
                &[LEFT, right]
            ),
            Some(1)
        );
        assert_eq!(monitor_for(LEFT, &[]), None);
    }
    #[test]
    fn center_resizes_both_dimensions_after_every_placement() {
        let expected = Rect {
            x: -1440,
            y: 340,
            width: 960,
            height: 520,
        };
        for previous in Action::POSITIONS {
            let current = previous.target(LEFT, LEFT);
            assert_eq!(
                Action::Center.target(LEFT, current),
                expected,
                "after {previous:?}"
            );
        }
        for size in [100, 9000] {
            assert_eq!(
                Action::Center.target(
                    LEFT,
                    Rect {
                        width: size,
                        height: size,
                        ..LEFT
                    }
                ),
                expected
            );
        }
        let odd = Rect {
            x: -1921,
            y: -1001,
            width: 1921,
            height: 1001,
        };
        let target = Action::Center.target(odd, odd);
        assert_eq!((target.width, target.height), (960, 500));
        let left = target.x - odd.x;
        let top = target.y - odd.y;
        assert!((left - (odd.width - target.width - left)).abs() <= 1);
        assert!((top - (odd.height - target.height - top)).abs() <= 1);
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    fn rect(x: i32, y: i32) -> Rect {
        Rect {
            x,
            y,
            width: 400,
            height: 400,
        }
    }
    #[test]
    fn visits_center_before_far_right_then_stops() {
        let left = rect(0, 400);
        let center = rect(400, 200);
        let right = rect(800, 400);
        assert_eq!(
            focus_neighbor(left, &[right, center], Action::FocusRight),
            Some(1)
        );
        assert_eq!(
            focus_neighbor(center, &[left, right], Action::FocusRight),
            Some(1)
        );
        assert_eq!(
            focus_neighbor(right, &[left, center], Action::FocusRight),
            None
        );
        assert_eq!(
            focus_neighbor(right, &[left, center], Action::FocusLeft),
            Some(1)
        );
    }
    #[test]
    fn directions_ignore_wrong_axis_and_identical_bounds() {
        let current = rect(-800, -400);
        let windows = [
            current,
            rect(-1200, -400),
            rect(-400, -400),
            rect(-800, -800),
            rect(-800, 0),
        ];
        for (action, expected) in [
            (Action::FocusLeft, 1),
            (Action::FocusRight, 2),
            (Action::FocusUp, 3),
            (Action::FocusDown, 4),
        ] {
            assert_eq!(focus_neighbor(current, &windows, action), Some(expected));
        }
        assert_eq!(
            focus_neighbor(current, &[current], Action::FocusRight),
            None
        );
        assert_eq!(focus_neighbor(current, &[], Action::FocusDown), None);
    }
    #[test]
    fn ties_keep_z_order_and_overlapping_windows_are_reachable() {
        let current = rect(0, 0);
        assert_eq!(
            focus_neighbor(current, &[rect(100, 0), rect(100, 0)], Action::FocusRight),
            Some(0)
        );
        assert_eq!(
            focus_neighbor(current, &[rect(10, 500)], Action::FocusRight),
            None
        );
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;

    #[test]
    fn adjacent_tiles_have_inner_spacing_and_outer_margins() {
        let area = Rect {
            x: -2001,
            y: 41,
            width: 2001,
            height: 1201,
        };
        for gaps in [
            SnapGaps::default(),
            SnapGaps {
                inner: 17,
                outer: 40,
            },
            SnapGaps { inner: 0, outer: 0 },
        ] {
            let left = gaps.inset(area, Action::TopLeft.target(area, area));
            let right = gaps.inset(area, Action::TopRight.target(area, area));
            let bottom = gaps.inset(area, Action::BottomLeft.target(area, area));
            assert_eq!(left.x - area.x, gaps.outer as i32);
            assert_eq!(left.y - area.y, gaps.outer as i32);
            assert_eq!(right.x - left.x - left.width, (2 * gaps.inner) as i32);
            assert_eq!(bottom.y - left.y - left.height, (2 * gaps.inner) as i32);
            assert_eq!(
                area.x + area.width - right.x - right.width,
                gaps.outer as i32
            );
            assert!(crate::is_edge_snapped(left, area, gaps));
            assert!(!crate::is_edge_snapped(
                Action::Center.target(area, area),
                area,
                gaps
            ));
        }
        let tiny = Rect {
            x: -3,
            y: 2,
            width: 3,
            height: 1,
        };
        let inset = SnapGaps {
            inner: 100,
            outer: 100,
        }
        .work_area(tiny);
        assert!(inset.width > 0 && inset.height > 0);
        assert_eq!(inset.intersection(tiny), Some(inset));
    }

    #[test]
    fn paired_resize_preserves_custom_spacing() {
        crate::set_snap_gaps(SnapGaps {
            inner: 30,
            outer: 20,
        });
        let top = Rect {
            x: 20,
            y: 20,
            width: 400,
            height: 300,
        };
        let bottom = Rect { y: 380, ..top };
        let (a, b) = resize_vertical_pair(top, bottom, true).unwrap();
        assert_eq!(b.y - a.y - a.height, 60);
        assert_eq!(a.y, top.y);
        assert_eq!(b.y + b.height, bottom.y + bottom.height);
        crate::set_snap_gaps(SnapGaps { inner: 0, outer: 0 });
    }
}
