//! Experimental animated snapping: frames between a window's current bounds
//! and its snap target. Pure geometry here; backends supply the native move.
use crate::geometry::Rect;
use anyhow::Result;
use std::time::{Duration, Instant};

/// Share of the duration a shrinking axis takes to reach its target size.
/// Fewer intermediate sizes mean fewer relayouts inside the application.
const SHRINK_SHARE: f64 = 0.4;
/// How strongly the middle slows: 0 is linear, 1 stops at the midpoint.
const MIDDLE_EASE: f64 = 0.6;

/// Fast-slow-fast progress: linear blended with a centered cubic, so motion
/// leaves and arrives quickly (about 2.2× linear) and eases through the middle.
pub fn progress(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let centered = t - 0.5;
    (1.0 - MIDDLE_EASE) * t + MIDDLE_EASE * (0.5 + 4.0 * centered * centered * centered)
}

fn lerp(from: i32, to: i32, p: f64) -> f64 {
    from as f64 + (to - from) as f64 * p
}

/// One axis (`start`, `size`) at time `t`. A shrinking size runs on a
/// compressed timeline; the window then sits inside the span the edges would
/// cover on the normal curve, anchored toward the edge that moves least.
fn axis(from: (i32, i32), to: (i32, i32), t: f64) -> (i32, i32) {
    let p = progress(t);
    let near = lerp(from.0, to.0, p);
    let far = lerp(from.0 + from.1, to.0 + to.1, p);
    if to.1 >= from.1 {
        return (near.round() as i32, (far - near).round() as i32);
    }
    let size = lerp(from.1, to.1, progress(t / SHRINK_SHARE));
    let moved_near = (to.0 - from.0).abs() as f64;
    let moved_far = ((to.0 + to.1) - (from.0 + from.1)).abs() as f64;
    let anchor = if moved_near + moved_far == 0.0 {
        0.5
    } else {
        moved_near / (moved_near + moved_far)
    };
    let start = near + (far - near - size) * anchor;
    // Round edges, not the size, so an anchored edge never jitters by a pixel.
    let (start, end) = (start.round() as i32, (start + size).round() as i32);
    (start, end - start)
}

/// The window's bounds at time `t` (0 to 1) of a move from `from` to `to`.
pub fn frame(from: Rect, to: Rect, t: f64) -> Rect {
    let (x, width) = axis((from.x, from.width), (to.x, to.width), t);
    let (y, height) = axis((from.y, from.height), (to.y, to.height), t);
    Rect {
        x,
        y,
        width: width.max(1),
        height: height.max(1),
    }
}

/// Drive intermediate frames for `duration`, then return so the caller makes
/// its usual exact final placement. Progress follows the clock, so a slow
/// application drops frames instead of stretching the animation. A failed
/// frame ends the animation early; the final placement still runs.
pub(crate) fn run(
    from: Rect,
    to: Rect,
    duration: Duration,
    step: &mut dyn FnMut(Rect) -> Result<()>,
    wait: &mut dyn FnMut(),
) {
    if from == to || from.width <= 0 || from.height <= 0 || duration.is_zero() {
        return;
    }
    let start = Instant::now();
    let mut last = from;
    loop {
        let t = start.elapsed().as_secs_f64() / duration.as_secs_f64();
        if t >= 1.0 {
            return;
        }
        let next = frame(from, to, t);
        if next != last {
            if step(next).is_err() {
                return;
            }
            last = next;
        }
        wait();
    }
}

/// Pace frames for backends without a compositor clock (about 120 Hz).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn pause() {
    std::thread::sleep(Duration::from_millis(8));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn curve_is_fast_slow_fast_and_monotonic() {
        assert_eq!(progress(0.0), 0.0);
        assert_eq!(progress(1.0), 1.0);
        assert!((progress(0.5) - 0.5).abs() < 1e-9);
        let speed = |t: f64| (progress(t + 0.01) - progress(t)) / 0.01;
        assert!(speed(0.0) > 2.0 && speed(0.98) > 2.0);
        assert!(speed(0.5) < 0.5);
        let mut previous = 0.0;
        for i in 1..=100 {
            let p = progress(i as f64 / 100.0);
            assert!(p >= previous);
            previous = p;
        }
    }

    #[test]
    fn frames_start_and_end_on_the_bounds() {
        let from = rect(100, 80, 700, 500);
        let to = rect(960, 0, 960, 1040);
        assert_eq!(frame(from, to, 0.0), from);
        assert_eq!(frame(from, to, 1.0), to);
        let shrink = rect(0, 0, 1920, 1040);
        let half = rect(960, 0, 960, 1040);
        assert_eq!(frame(shrink, half, 0.0), shrink);
        assert_eq!(frame(shrink, half, 1.0), half);
    }

    #[test]
    fn shrinking_reaches_its_size_early() {
        let from = rect(0, 0, 1920, 1040);
        let to = rect(0, 0, 960, 520);
        let early = frame(from, to, SHRINK_SHARE);
        assert_eq!((early.width, early.height), (960, 520));
        // Growing keeps the normal curve.
        let back = frame(to, from, SHRINK_SHARE);
        assert!(back.width < 1920 && back.height < 1040);
    }

    #[test]
    fn shrinking_anchors_the_still_edge_and_never_overshoots() {
        // Full width to the right half: the right edge stays put.
        let from = rect(0, 0, 1920, 1040);
        let to = rect(960, 0, 960, 1040);
        for i in 0..=100 {
            let f = frame(from, to, i as f64 / 100.0);
            assert_eq!(f.x + f.width, 1920, "t={i}");
            assert!(f.width >= 960);
        }
        // Moving while shrinking stays within the hull and never undersizes.
        let from = rect(0, 100, 800, 900);
        let to = rect(1000, 0, 400, 500);
        for i in 0..=100 {
            let f = frame(from, to, i as f64 / 100.0);
            assert!(f.width >= 400 && f.width <= 800, "t={i} {f:?}");
            assert!(f.x >= 0 && f.x + f.width <= 1400, "t={i} {f:?}");
        }
    }

    #[test]
    fn run_skips_no_op_moves_and_stops_on_a_failed_frame() {
        let bounds = rect(0, 0, 100, 100);
        let mut frames = 0;
        run(
            bounds,
            bounds,
            Duration::from_millis(50),
            &mut |_| {
                frames += 1;
                Ok(())
            },
            &mut || {},
        );
        assert_eq!(frames, 0);
        run(
            bounds,
            rect(500, 0, 100, 100),
            Duration::from_secs(5),
            &mut |_| {
                frames += 1;
                anyhow::bail!("closed")
            },
            &mut || std::thread::sleep(Duration::from_millis(1)),
        );
        assert_eq!(frames, 1);
    }

    #[test]
    fn run_finishes_within_its_duration() {
        let start = Instant::now();
        let mut frames = Vec::new();
        run(
            rect(0, 0, 800, 600),
            rect(960, 0, 960, 1040),
            // Long enough that a CI runner oversleeping each 1 ms wait (macOS
            // often sleeps 15 ms or more) still yields several frames.
            Duration::from_millis(400),
            &mut |f| {
                frames.push(f);
                Ok(())
            },
            &mut || std::thread::sleep(Duration::from_millis(1)),
        );
        assert!(start.elapsed() < Duration::from_millis(1500));
        assert!(frames.len() > 3);
        assert!(frames.windows(2).all(|w| w[0].x <= w[1].x));
    }
}
