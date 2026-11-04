//! Safe interfaces to tt-pane native window operations.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod geometry;
mod platform;
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
