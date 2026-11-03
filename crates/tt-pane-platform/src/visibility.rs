//! Rectangle-based occlusion in desktop coordinates, independent of native APIs.
use crate::geometry::Rect;

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

pub(crate) fn focus_visible(
    window: Rect,
    screens: &[Rect],
    above: &[Rect],
    include_obscured: bool,
) -> bool {
    if window.width <= 0 || window.height <= 0 {
        return false;
    }
    if include_obscured {
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
    visible.iter().map(|r| r.area()).sum::<i128>() * 2 >= window.area()
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
    fn exact_half_is_included_but_less_is_not() {
        for (covered, expected) in [(0, true), (49, true), (50, true), (51, false), (100, false)] {
            assert_eq!(
                focus_visible(WINDOW, &[WINDOW], &[r(0, 0, covered, 100)], false),
                expected
            );
        }
    }

    #[test]
    fn overlapping_covers_are_not_counted_twice() {
        let covers = [r(0, 0, 30, 100), r(20, 0, 30, 100)];
        assert!(focus_visible(WINDOW, &[WINDOW], &covers, false));
        assert!(!focus_visible(
            WINDOW,
            &[WINDOW],
            &[covers[0], r(30, 0, 21, 100)],
            false
        ));
    }

    #[test]
    fn offscreen_area_and_monitor_gaps_are_not_visible() {
        assert!(focus_visible(WINDOW, &[r(0, 0, 50, 100)], &[], false));
        assert!(!focus_visible(WINDOW, &[r(0, 0, 49, 100)], &[], false));
        assert!(!focus_visible(
            WINDOW,
            &[r(0, 0, 25, 100), r(75, 0, 25, 100)],
            &[r(0, 0, 1, 100)],
            false
        ));
        assert!(!focus_visible(WINDOW, &[r(0, 0, 40, 100); 2], &[], false));
        assert!(focus_visible(
            r(-100, -100, 200, 100),
            &[r(-100, -100, 100, 100), r(0, -100, 100, 100)],
            &[],
            false
        ));
    }

    #[test]
    fn toggle_bypasses_occlusion_and_offscreen_filter() {
        assert!(focus_visible(WINDOW, &[WINDOW], &[WINDOW], true));
        assert!(focus_visible(WINDOW, &[], &[], true));
        assert!(!focus_visible(r(0, 0, 0, 100), &[WINDOW], &[], true));
    }
}
