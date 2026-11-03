//! Rectangle-based occlusion in desktop coordinates, independent of native APIs.
use crate::{FocusVisibility, geometry::Rect};

#[derive(Clone, Copy)]
struct Bounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Bounds {
    fn from_rect(r: Rect) -> Self {
        Self {
            left: r.x.into(),
            top: r.y.into(),
            right: i64::from(r.x) + i64::from(r.width),
            bottom: i64::from(r.y) + i64::from(r.height),
        }
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let r = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (r.right > r.left && r.bottom > r.top).then_some(r)
    }

    fn area(self) -> i128 {
        i128::from(self.right - self.left) * i128::from(self.bottom - self.top)
    }

    fn subtract(self, cover: Self, output: &mut Vec<Self>) {
        let Some(hit) = self.intersection(cover) else {
            output.push(self);
            return;
        };
        for r in [
            Self {
                bottom: hit.top,
                ..self
            },
            Self {
                top: hit.bottom,
                ..self
            },
            Self {
                top: hit.top,
                bottom: hit.bottom,
                right: hit.left,
                ..self
            },
            Self {
                top: hit.top,
                bottom: hit.bottom,
                left: hit.right,
                ..self
            },
        ] {
            if r.right > r.left && r.bottom > r.top {
                output.push(r);
            }
        }
    }
}

/// Edge placements remain navigable even when covered. Use live work-area
/// alignment so native OS snaps and resized tiles qualify without stale history.
pub(crate) fn focus_eligible(
    window: Rect,
    work_area: Option<Rect>,
    screens: &[Rect],
    above: &[Rect],
    visibility: FocusVisibility,
) -> bool {
    focus_visible(
        window,
        screens,
        above,
        if is_snapped(window, work_area) {
            FocusVisibility::IncludeHidden
        } else {
            visibility
        },
    )
}

pub(crate) fn is_snapped(window: Rect, work_area: Option<Rect>) -> bool {
    work_area.is_some_and(|area| {
        if window.width <= 0 || window.height <= 0 || area.width <= 0 || area.height <= 0 {
            return false;
        }
        let w = Bounds::from_rect(window);
        let a = Bounds::from_rect(area);
        let contained = w.left >= a.left - 2
            && w.top >= a.top - 2
            && w.right <= a.right + 2
            && w.bottom <= a.bottom + 2;
        contained
            && [
                (w.left - a.left),
                (w.top - a.top),
                (w.right - a.right),
                (w.bottom - a.bottom),
            ]
            .into_iter()
            .any(|distance| distance.abs() <= 2)
    })
}

fn focus_visible(
    window: Rect,
    screens: &[Rect],
    above: &[Rect],
    visibility: FocusVisibility,
) -> bool {
    if window.width <= 0 || window.height <= 0 {
        return false;
    }
    if visibility == FocusVisibility::IncludeHidden {
        return true;
    }
    let window = Bounds::from_rect(window);
    let mut offscreen = vec![window];
    let mut visible = Vec::new();
    // Subtract each screen from the remainder to avoid counting mirrored displays twice.
    for &screen in screens {
        let screen = Bounds::from_rect(screen);
        let mut remaining = Vec::new();
        for piece in offscreen {
            if let Some(hit) = piece.intersection(screen) {
                visible.push(hit);
            }
            piece.subtract(screen, &mut remaining);
        }
        offscreen = remaining;
    }
    // Each subtraction leaves disjoint pieces, so overlapping covers count only once.
    for &cover in above {
        let mut remaining = Vec::new();
        for piece in visible {
            piece.subtract(Bounds::from_rect(cover), &mut remaining);
        }
        visible = remaining;
    }
    let visible_area = visible.iter().map(|r| r.area()).sum::<i128>();
    match visibility {
        FocusVisibility::MoreThanPercent(percent) => {
            visible_area * 100 > window.area() * i128::from(percent)
        }
        FocusVisibility::AtLeastHalf => visible_area * 2 >= window.area(),
        FocusVisibility::IncludeHidden => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    const WINDOW: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    };

    #[test]
    fn taps_use_configured_threshold_and_holds_reach_covered_middle_windows() {
        use crate::geometry::{Action, focus_neighbor};
        let area = r(0, 0, 1000, 1000);
        let middle = Action::Center.target(area, area);
        let left = Action::Left.target(area, area);
        let right = Action::Right.target(area, area);
        for percent in [0, 10, 25, 50, 99] {
            // At the boundary the middle window is excluded; one pixel more qualifies.
            for visible_width in [
                middle.width * percent as i32 / 100,
                middle.width * percent as i32 / 100 + 1,
            ] {
                let cover = r(
                    middle.x + visible_width,
                    middle.y,
                    middle.width - visible_width,
                    middle.height,
                );
                let eligible = focus_eligible(
                    middle,
                    Some(area),
                    &[area],
                    &[cover],
                    FocusVisibility::MoreThanPercent(percent),
                );
                assert_eq!(
                    eligible,
                    visible_width * 100 > middle.width * percent as i32
                );
                for (source, action, edge) in [
                    (left, Action::FocusRight, right),
                    (right, Action::FocusLeft, left),
                ] {
                    let candidates: Vec<_> = [middle, edge]
                        .into_iter()
                        .filter(|rect| {
                            focus_eligible(
                                *rect,
                                Some(area),
                                &[area],
                                &[cover],
                                FocusVisibility::MoreThanPercent(percent),
                            )
                        })
                        .collect();
                    let target = candidates[focus_neighbor(source, &candidates, action).unwrap()];
                    assert_eq!(target, if eligible { middle } else { edge });
                }
            }
            assert!(!focus_eligible(
                middle,
                Some(area),
                &[area],
                &[area],
                FocusVisibility::MoreThanPercent(percent)
            ));
            assert!(focus_eligible(
                middle,
                Some(area),
                &[area],
                &[area],
                FocusVisibility::IncludeHidden
            ));
        }
        assert!(!focus_eligible(
            middle,
            Some(area),
            &[],
            &[],
            FocusVisibility::MoreThanPercent(10)
        ));
    }

    #[test]
    fn covered_snaps_remain_eligible_but_middle_windows_require_half_visible() {
        use crate::geometry::Action;
        let area = r(-1600, 40, 1600, 1000);
        for action in Action::POSITIONS {
            let window = action.target(area, area);
            assert_eq!(
                focus_eligible(
                    window,
                    Some(area),
                    &[area],
                    &[area],
                    FocusVisibility::AtLeastHalf
                ),
                action != Action::Center,
                "{action:?}"
            );
        }
        let middle = Action::Center.target(area, area);
        assert!(focus_eligible(
            middle,
            Some(area),
            &[area],
            &[r(middle.x, middle.y, middle.width / 2, middle.height)],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_eligible(
            middle,
            Some(area),
            &[area],
            &[r(middle.x, middle.y, middle.width / 2 + 1, middle.height)],
            FocusVisibility::AtLeastHalf
        ));
        assert!(focus_eligible(
            middle,
            Some(area),
            &[area],
            &[area],
            FocusVisibility::IncludeHidden
        ));
    }

    #[test]
    fn resized_edge_tiles_qualify_until_moved_into_the_middle() {
        let area = r(-1600, 40, 1600, 1000);
        let edge = r(-1600, 140, 333, 400);
        assert!(focus_eligible(
            edge,
            Some(area),
            &[area],
            &[area],
            FocusVisibility::AtLeastHalf
        ));
        assert!(focus_eligible(
            Rect {
                x: edge.x + 2,
                ..edge
            },
            Some(area),
            &[area],
            &[area],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_eligible(
            Rect {
                x: edge.x + 3,
                ..edge
            },
            Some(area),
            &[area],
            &[area],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_eligible(
            Rect {
                x: edge.x - 20,
                ..edge
            },
            Some(area),
            &[area],
            &[area],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_eligible(
            edge,
            None,
            &[area],
            &[area],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_eligible(
            Rect { width: 0, ..edge },
            Some(area),
            &[area],
            &[],
            FocusVisibility::AtLeastHalf
        ));
    }

    #[test]
    fn exact_half_is_included_but_less_is_not() {
        for (covered, expected) in [(0, true), (49, true), (50, true), (51, false), (100, false)] {
            assert_eq!(
                focus_visible(
                    WINDOW,
                    &[WINDOW],
                    &[r(0, 0, covered, 100)],
                    FocusVisibility::AtLeastHalf
                ),
                expected
            );
        }
    }

    #[test]
    fn overlapping_covers_are_not_counted_twice() {
        let covers = [r(0, 0, 30, 100), r(20, 0, 30, 100)];
        assert!(focus_visible(
            WINDOW,
            &[WINDOW],
            &covers,
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_visible(
            WINDOW,
            &[WINDOW],
            &[covers[0], r(30, 0, 21, 100)],
            FocusVisibility::AtLeastHalf
        ));
    }

    #[test]
    fn offscreen_area_and_monitor_gaps_are_not_visible() {
        assert!(focus_visible(
            WINDOW,
            &[r(0, 0, 50, 100)],
            &[],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_visible(
            WINDOW,
            &[r(0, 0, 49, 100)],
            &[],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_visible(
            WINDOW,
            &[r(0, 0, 25, 100), r(75, 0, 25, 100)],
            &[r(0, 0, 1, 100)],
            FocusVisibility::AtLeastHalf
        ));
        assert!(!focus_visible(
            WINDOW,
            &[r(0, 0, 40, 100); 2],
            &[],
            FocusVisibility::AtLeastHalf
        ));
        assert!(focus_visible(
            r(-100, -100, 200, 100),
            &[r(-100, -100, 100, 100), r(0, -100, 100, 100)],
            &[],
            FocusVisibility::AtLeastHalf
        ));
    }

    #[test]
    fn toggle_bypasses_occlusion_and_offscreen_filter() {
        assert!(focus_visible(
            WINDOW,
            &[WINDOW],
            &[WINDOW],
            FocusVisibility::IncludeHidden
        ));
        assert!(focus_visible(
            WINDOW,
            &[],
            &[],
            FocusVisibility::IncludeHidden
        ));
        assert!(!focus_visible(
            r(0, 0, 0, 100),
            &[WINDOW],
            &[],
            FocusVisibility::IncludeHidden
        ));
    }
}
