//! Edge + GC-phase-aware coverage tracking.
//!
//! All bitmap types share a contiguous buffer layout: edge bytes first, then GC
//! bytes. Slice algorithms (`has_new_bits_slice`, `merge_slice`, `count_nonzero`,
//! `classify_slice`) live as free fns in bitmap.rs; atomic variants in atomic.rs.
//! All cross-type operations `assert_eq!` on lengths rather than silently
//! truncating, since a size mismatch is always a bug.
//!
//! `SharedCoverage` maps one shm region (edge+gc contiguous). `open()` validates
//! the mapped size matches the expected size.

mod atomic;
mod bitmap;
mod shared;

pub use atomic::AtomicBitmap;
pub use bitmap::CoverageBitmap;
pub use shared::SharedCoverage;

pub const DEFAULT_BITMAP_SIZE: usize = 64 * 1024;
