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
}

impl Action {
    pub const ALL: [Self; 10] = [
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
        }
    }

    pub fn target(self, area: Rect, _current: Rect) -> Rect {
        let half_w = area.width / 2;
        let half_h = area.height / 2;
        let mut target = area;
        match self {
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
    const LEFT: Rect = Rect {
        x: -1920,
        y: 80,
        width: 1920,
        height: 1040,
    };
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
        for previous in Action::ALL {
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
