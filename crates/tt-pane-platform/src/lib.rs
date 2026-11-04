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
