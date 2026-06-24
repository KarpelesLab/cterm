//! Mouse reporting utilities.
//!
//! The implementation now lives in `cterm-core` so all platform front-ends share
//! one encoder. Re-exported here to keep existing `crate::mouse::…` references working.

pub use cterm_core::mouse::*;
