use std::sync::atomic::{AtomicU8, Ordering};

use crate::bitmap::CoverageBitmap;

pub struct AtomicBitmap {
    buf: Vec<AtomicU8>,
    edge_len: usize,
}

impl AtomicBitmap {
    pub fn new(edge_size: usize, gc_size: usize) -> Self {
        Self {
            buf: (0..edge_size + gc_size).map(|_| AtomicU8::new(0)).collect(),
            edge_len: edge_size,
        }
    }

    fn edge_atoms(&self) -> &[AtomicU8] {
        &self.buf[..self.edge_len]
    }

    fn gc_atoms(&self) -> &[AtomicU8] {
        &self.buf[self.edge_len..]
    }

    pub fn has_new_bits(&self, local: &CoverageBitmap) -> bool {
        has_new_bits_atomic(local.edge_bytes(), self.edge_atoms())
            || has_new_bits_atomic(local.gc_bytes(), self.gc_atoms())
    }

    pub fn merge(&self, local: &CoverageBitmap) {
        merge_atomic(local.edge_bytes(), self.edge_atoms());
        merge_atomic(local.gc_bytes(), self.gc_atoms());
    }

    /// Atomically check for new bits and merge if found. Returns true if
    /// any new coverage was discovered. This avoids the TOCTOU race of
    /// separate `has_new_bits` + `merge` calls.
    pub fn merge_if_new(&self, local: &CoverageBitmap) -> bool {
        let new_edge = merge_if_new_atomic(local.edge_bytes(), self.edge_atoms());
        let new_gc = merge_if_new_atomic(local.gc_bytes(), self.gc_atoms());
        new_edge || new_gc
    }

    /// Merge all bits and return whether new *edge* bits were found.
    /// GC bits are always merged (for tracking) but only new edge coverage
    /// triggers corpus growth — this prevents GC-timing noise from bloating
    /// the corpus.
    pub fn merge_if_new_edge(&self, local: &CoverageBitmap) -> bool {
        let new_edge = merge_if_new_atomic(local.edge_bytes(), self.edge_atoms());
        // Always merge GC bits for tracking, but don't let them drive corpus growth.
        merge_atomic(local.gc_bytes(), self.gc_atoms());
        new_edge
    }

    /// Like `merge_if_new_edge`, but only considers a truly new edge
    /// (byte going from 0→nonzero) as novel — ignores hit-count bucket
    /// changes on already-known edges. This prevents corpus bloat from
    /// the same edges being hit with slightly different iteration counts.
    pub fn merge_if_new_edge_strict(&self, local: &CoverageBitmap) -> bool {
        let new_edge = merge_new_edge_strict(local.edge_bytes(), self.edge_atoms());
        merge_atomic(local.gc_bytes(), self.gc_atoms());
        new_edge
    }

    pub fn snapshot(&self) -> CoverageBitmap {
        let bytes: Vec<u8> = self.buf.iter().map(|a| a.load(Ordering::Relaxed)).collect();
        CoverageBitmap::from_raw(bytes, self.edge_len)
    }
}

fn has_new_bits_atomic(local: &[u8], global: &[AtomicU8]) -> bool {
    assert_eq!(
        local.len(),
        global.len(),
        "bitmap size mismatch in has_new_bits_atomic"
    );
    for (l, g) in local.iter().zip(global.iter()) {
        if l & !g.load(Ordering::Relaxed) != 0 {
            return true;
        }
    }
    false
}

fn merge_atomic(src: &[u8], dst: &[AtomicU8]) {
    assert_eq!(src.len(), dst.len(), "bitmap size mismatch in merge_atomic");
    for (s, d) in src.iter().zip(dst.iter()) {
        if *s != 0 {
            d.fetch_or(*s, Ordering::Relaxed);
        }
    }
}

/// Merge src into dst, returning true only if any byte transitioned from
/// 0 → nonzero. Ignores new bits added to already-nonzero bytes (i.e.
/// hit-count bucket changes on known edges). All bits are still merged.
fn merge_new_edge_strict(src: &[u8], dst: &[AtomicU8]) -> bool {
    assert_eq!(
        src.len(),
        dst.len(),
        "bitmap size mismatch in merge_new_edge_strict"
    );
    let mut found_new = false;
    for (s, d) in src.iter().zip(dst.iter()) {
        if *s != 0 {
            let old = d.fetch_or(*s, Ordering::Relaxed);
            if old == 0 {
                found_new = true;
            }
        }
    }
    found_new
}

/// Merge src into dst, returning true if any genuinely new bits were set.
/// Each byte is merged with fetch_or; we detect novelty by comparing the
/// old value.
fn merge_if_new_atomic(src: &[u8], dst: &[AtomicU8]) -> bool {
    assert_eq!(
        src.len(),
        dst.len(),
        "bitmap size mismatch in merge_if_new_atomic"
    );
    let mut found_new = false;
    for (s, d) in src.iter().zip(dst.iter()) {
        if *s != 0 {
            let old = d.fetch_or(*s, Ordering::Relaxed);
            if *s & !old != 0 {
                found_new = true;
            }
        }
    }
    found_new
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn concurrent_merge_overlapping_regions() {
        let atomic = Arc::new(AtomicBitmap::new(16, 16));

        let thread_data: Vec<CoverageBitmap> = vec![
            {
                let mut bm = CoverageBitmap::new(16, 16);
                for b in &mut bm.edge_bytes_mut()[0..8] {
                    *b = 0x01;
                }
                bm
            },
            {
                let mut bm = CoverageBitmap::new(16, 16);
                for b in &mut bm.edge_bytes_mut()[0..8] {
                    *b = 0x02;
                }
                bm
            },
            {
                let mut bm = CoverageBitmap::new(16, 16);
                for b in &mut bm.edge_bytes_mut()[4..12] {
                    *b = 0x04;
                }
                bm
            },
            {
                let mut bm = CoverageBitmap::new(16, 16);
                for b in &mut bm.gc_bytes_mut()[0..8] {
                    *b = 0x80;
                }
                bm
            },
        ];

        let handles: Vec<_> = thread_data
            .into_iter()
            .map(|local| {
                let a = Arc::clone(&atomic);
                thread::spawn(move || {
                    a.merge(&local);
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let snap = atomic.snapshot();

        for i in 0..4 {
            assert_eq!(snap.edge_bytes()[i], 0x03, "edge byte {i}");
        }
        for i in 4..8 {
            assert_eq!(snap.edge_bytes()[i], 0x07, "edge byte {i}");
        }
        for i in 8..12 {
            assert_eq!(snap.edge_bytes()[i], 0x04, "edge byte {i}");
        }
        for i in 0..8 {
            assert_eq!(snap.gc_bytes()[i], 0x80, "gc byte {i}");
        }
    }

    #[test]
    fn has_new_bits_consistent_with_non_atomic() {
        let mut local = CoverageBitmap::new(16, 16);
        local.edge_bytes_mut()[3] = 0x0F;
        local.gc_bytes_mut()[7] = 0x80;

        let mut global_plain = CoverageBitmap::new(16, 16);
        global_plain.edge_bytes_mut()[3] = 0x03;

        let atomic = AtomicBitmap::new(16, 16);
        atomic.buf[3].store(0x03, Ordering::Relaxed);

        assert_eq!(
            local.has_new_bits(&global_plain),
            atomic.has_new_bits(&local)
        );

        // subset case: no new bits
        let mut subset = CoverageBitmap::new(16, 16);
        subset.edge_bytes_mut()[3] = 0x03;
        atomic.merge(&local);
        assert!(!atomic.has_new_bits(&subset));
    }

    #[test]
    fn merge_if_new_novelty_detection() {
        let atomic = AtomicBitmap::new(16, 16);

        let mut local = CoverageBitmap::new(16, 16);
        local.edge_bytes_mut()[0] = 0x03;
        assert!(atomic.merge_if_new(&local), "novel bits should return true");
        assert_eq!(atomic.snapshot().edge_bytes()[0], 0x03);

        let mut subset = CoverageBitmap::new(16, 16);
        subset.edge_bytes_mut()[0] = 0x01;
        assert!(!atomic.merge_if_new(&subset), "subset should return false");
    }

    #[test]
    fn merge_if_new_concurrent_no_lost_bits() {
        let atomic = Arc::new(AtomicBitmap::new(64, 0));
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let a = Arc::clone(&atomic);
                thread::spawn(move || {
                    let mut local = CoverageBitmap::new(64, 0);
                    local.edge_bytes_mut()[t] = 0xFF;
                    a.merge_if_new(&local);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let snap = atomic.snapshot();
        for i in 0..8 {
            assert_eq!(snap.edge_bytes()[i], 0xFF, "byte {i} should be set");
        }
    }

    #[test]
    #[should_panic(expected = "bitmap size mismatch")]
    fn size_mismatch_panics() {
        let local = CoverageBitmap::new(16, 16);
        let atomic = AtomicBitmap::new(32, 32);
        atomic.has_new_bits(&local);
    }
}
