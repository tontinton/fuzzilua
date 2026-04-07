mod atomic;
mod bitmap;
mod shared;

pub use atomic::AtomicBitmap;
pub use bitmap::CoverageBitmap;
pub use shared::SharedCoverage;

pub const DEFAULT_BITMAP_SIZE: usize = 64 * 1024;
