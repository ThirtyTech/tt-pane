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

    pub const ALL: [Self; 13] = [
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
    ];

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
            Self::ShrinkWidth | Self::ExpandWidth | Self::FillGap => None,
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
        }
    }

    pub fn target(self, area: Rect, current: Rect) -> Rect {
        let half_w = area.width / 2;
        let half_h = area.height / 2;
        let mut target = area;
        match self {
            // Contextual action: the coordinator resolves its saved rectangle.
            Self::FillGap => return current,
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

#[cfg(test)]
mod tests {
    use super::*;
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
