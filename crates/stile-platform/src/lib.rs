//! Safe interfaces to sTile native window operations.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod geometry;
mod platform;
pub use platform::*;

#[cfg(all(windows, feature = "test-support"))]
#[allow(unsafe_code)]
pub mod test_support;
