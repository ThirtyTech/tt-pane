//! Safe interfaces to tt-pane native window operations.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod animation;
pub mod geometry;
mod platform;
pub mod tiling;
#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
mod visibility;
pub use platform::*;

#[cfg(all(windows, feature = "test-support"))]
#[allow(unsafe_code)]
pub mod test_support;

thread_local! {
    static SNAP_GAPS: std::cell::Cell<geometry::SnapGaps> = const {
        std::cell::Cell::new(geometry::SnapGaps { inner: 0, outer: 0 })
    };
    static SNAP_ANIMATION: std::cell::Cell<Option<std::time::Duration>> =
        const { std::cell::Cell::new(None) };
}

/// Animate single-window snaps made by `run` on this thread over `duration`
/// (experimental). Other moves, such as tiling and group arrangements, stay
/// instant. The previous setting is restored afterwards, even on unwinding.
pub fn with_snap_animation<T>(duration: Option<std::time::Duration>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<std::time::Duration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SNAP_ANIMATION.set(self.0);
        }
    }
    let _restore = Restore(SNAP_ANIMATION.replace(duration));
    run()
}

#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
pub(crate) fn snap_animation() -> Option<std::time::Duration> {
    SNAP_ANIMATION.get()
}

/// Set layout spacing for synchronous native operations on this calling thread.
pub fn set_snap_gaps(gaps: geometry::SnapGaps) {
    SNAP_GAPS.set(gaps);
}

pub fn is_edge_snapped(
    window: geometry::Rect,
    area: geometry::Rect,
    gaps: geometry::SnapGaps,
) -> bool {
    visibility::is_snapped_with_gaps(window, Some(area), gaps)
}
