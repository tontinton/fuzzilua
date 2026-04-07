//! Edge + GC-phase-aware coverage tracking.
//!
//! All bitmap types share a single contiguous buffer layout: edge bytes first,
//! then GC bytes (same layout in `CoverageBitmap`, `AtomicBitmap`, `SharedCoverage`).
//!
//! Slice algorithms (`has_new_bits`, `merge`, `count_nonzero`, `classify`) live as
//! free fns in bitmap.rs and are reused by atomic.rs. All cross-type operations
//! `assert_eq!` on lengths, never silently truncate with `.min()`, because a size
//! mismatch is always a programming bug.
//!
//! `SharedCoverage` uses ONE shm region (edge+gc contiguous): one name, one mmap,
//! one Drop. Its `open()` validates the mapped size matches expected size.

mod atomic;
mod bitmap;
mod shared;

pub use atomic::AtomicBitmap;
pub use bitmap::CoverageBitmap;
pub use shared::SharedCoverage;

pub const DEFAULT_BITMAP_SIZE: usize = 64 * 1024;
