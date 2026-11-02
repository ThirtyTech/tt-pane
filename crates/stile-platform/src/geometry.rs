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

    pub fn target(self, area: Rect, current: Rect) -> Rect {
        let half_w = area.width / 2;
        let half_h = area.height / 2;
        let mut target = area;
        match self {
            Self::Center => {
                target.width = current.width.min(area.width);
                target.height = current.height.min(area.height);
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
    fn center_preserves_size_and_clamps_oversized_windows() {
        let current = Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            Action::Center.target(LEFT, current),
            Rect {
                x: -1360,
                y: 300,
                width: 800,
                height: 600
            }
        );
        assert_eq!(
            Action::Center.target(
                LEFT,
                Rect {
                    width: 9000,
                    height: 9000,
                    ..current
                }
            ),
            LEFT
        );
    }
}
