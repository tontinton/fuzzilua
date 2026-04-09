mod error;
mod minimize;
mod persist;
mod scheduler;

pub use error::CorpusError;
pub use minimize::minimize;
pub use persist::save_crash;
pub use scheduler::{CorpusScheduler, FocusedScheduler, UniformScheduler, WeightedScheduler};

use fuzzilua_coverage::CoverageBitmap;
use fuzzilua_ir::Program;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusEntry {
    pub program: Program,
    pub coverage: CoverageBitmap,
    pub mutation_count: u32,
    #[serde(default)]
    pub cached_nonzero: u32,
}

pub struct Corpus {
    entries: Vec<CorpusEntry>,
    global_coverage: CoverageBitmap,
    scheduler: Box<dyn CorpusScheduler>,
    dir: PathBuf,
}

impl Corpus {
    pub fn new(
        scheduler: Box<dyn CorpusScheduler>,
        dir: impl Into<PathBuf>,
        edge_size: usize,
        gc_size: usize,
    ) -> Self {
        Self {
            entries: Vec::new(),
            global_coverage: CoverageBitmap::new(edge_size, gc_size),
            scheduler,
            dir: dir.into(),
        }
    }

    pub fn add(&mut self, program: Program, coverage: CoverageBitmap) -> bool {
        if !coverage.has_new_bits(&self.global_coverage) {
            return false;
        }

        coverage.merge_into(&mut self.global_coverage);

        let entry = CorpusEntry {
            cached_nonzero: coverage.total_nonzero(),
            program,
            coverage,
            mutation_count: 0,
        };

        if let Err(e) = persist::save_entry(&self.dir, &entry) {
            warn!("failed to persist corpus entry: {e}");
        }

        self.entries.push(entry);

        debug!(corpus_size = self.entries.len(), "added new corpus entry");
        true
    }

    pub fn select(&self, rng: &mut impl Rng) -> &CorpusEntry {
        assert!(!self.entries.is_empty(), "cannot select from empty corpus");
        let idx = self.scheduler.select(&self.entries, rng);
        &self.entries[idx]
    }

    /// Select an entry, increment its mutation count, and return a clone of its program.
    /// Requires `&mut self` — callers should use a write lock.
    pub fn select_and_track(&mut self, rng: &mut impl Rng) -> Program {
        assert!(!self.entries.is_empty(), "cannot select from empty corpus");
        let idx = self.scheduler.select(&self.entries, rng);
        self.entries[idx].mutation_count = self.entries[idx].mutation_count.saturating_add(1);
        self.entries[idx].program.clone()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_coverage(&self) -> (u32, u32) {
        self.global_coverage.count_bits()
    }

    pub fn bitmap_sizes(&self) -> (usize, usize) {
        (
            self.global_coverage.edge_len(),
            self.global_coverage.gc_len(),
        )
    }

    /// Add an entry whose coverage has already been validated externally
    /// (e.g. by `AtomicBitmap::has_new_bits`). Merges coverage into the
    /// internal global bitmap and persists, but skips the internal
    /// `has_new_bits` gate. Use this from multi-worker paths where the
    /// atomic bitmap is the single source of truth for novelty.
    pub fn add_unchecked(&mut self, program: Program, coverage: CoverageBitmap) {
        coverage.merge_into(&mut self.global_coverage);
        let entry = CorpusEntry {
            cached_nonzero: coverage.total_nonzero(),
            program,
            coverage,
            mutation_count: 0,
        };
        if let Err(e) = persist::save_entry(&self.dir, &entry) {
            warn!("failed to persist corpus entry: {e}");
        }
        self.entries.push(entry);
    }
    pub fn add_blind(&mut self, program: Program) {
        let coverage = CoverageBitmap::new(
            self.global_coverage.edge_len(),
            self.global_coverage.gc_len(),
        );
        let entry = CorpusEntry {
            cached_nonzero: 0,
            program,
            coverage,
            mutation_count: 0,
        };
        if let Err(e) = persist::save_entry(&self.dir, &entry) {
            warn!("failed to persist blind corpus entry: {e}");
        }
        self.entries.push(entry);
    }

    /// Snapshot coverage data for non-blocking compaction.
    pub fn snapshot_coverage(&self) -> Vec<(u32, CoverageBitmap)> {
        self.entries
            .iter()
            .map(|e| (e.cached_nonzero, e.coverage.clone()))
            .collect()
    }

    /// Apply a pre-computed eviction mask (from `compute_eviction`).
    /// The mask may be stale if workers added entries between snapshot and
    /// eviction — in that case we only apply to the prefix we have data for.
    pub fn apply_eviction(&mut self, keep: &[bool]) {
        let before = self.entries.len();
        let keep_len = keep.len();
        if keep_len > before {
            // Corpus shrank (shouldn't happen), bail out.
            return;
        }

        let mut idx = 0;
        self.entries.retain(|_| {
            // Entries added after the snapshot are always kept.
            let k = if idx < keep_len { keep[idx] } else { true };
            idx += 1;
            k
        });

        let evicted = before - self.entries.len();
        if evicted > 0 {
            if let Err(e) = self.re_persist() {
                warn!("failed to re-persist corpus after compaction: {e}");
            }
            info!(evicted, remaining = self.entries.len(), "corpus compacted");
        }
    }

    pub fn compact(&mut self) {
        let before = self.entries.len();
        let mut keep = vec![true; before];

        for i in 0..before {
            if !keep[i] {
                continue;
            }
            let nz_i = self.entries[i].cached_nonzero;
            for j in (i + 1)..before {
                if !keep[j] {
                    continue;
                }
                let nz_j = self.entries[j].cached_nonzero;
                // Quick check: if J has more nonzero bits than I, J can't be
                // a subset of I (and vice versa). Skip the expensive bitmap
                // scan for the impossible direction.
                let j_sub_i = nz_j <= nz_i
                    && self.entries[j]
                        .coverage
                        .is_subset_of(&self.entries[i].coverage);
                let i_sub_j = nz_i <= nz_j
                    && self.entries[i]
                        .coverage
                        .is_subset_of(&self.entries[j].coverage);
                if j_sub_i && !i_sub_j {
                    keep[j] = false;
                } else if i_sub_j && !j_sub_i {
                    keep[i] = false;
                    break;
                }
            }
        }

        let mut idx = 0;
        self.entries.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });

        let evicted = before - self.entries.len();
        if evicted > 0 {
            if let Err(e) = self.re_persist() {
                warn!("failed to re-persist corpus after compaction: {e}");
            }
            info!(evicted, remaining = self.entries.len(), "corpus compacted");
        }
    }

    pub fn load(
        scheduler: Box<dyn CorpusScheduler>,
        dir: impl Into<PathBuf>,
        edge_size: usize,
        gc_size: usize,
    ) -> Result<Self, CorpusError> {
        let dir = dir.into();
        let mut corpus = Self::new(scheduler, &dir, edge_size, gc_size);

        if !dir.exists() {
            return Ok(corpus);
        }

        let entries = persist::load_entries(&dir)?;
        for entry in entries {
            entry.coverage.merge_into(&mut corpus.global_coverage);
            corpus.entries.push(entry);
        }

        info!(loaded = corpus.entries.len(), "loaded corpus from disk");
        Ok(corpus)
    }

    fn re_persist(&self) -> Result<(), CorpusError> {
        persist::remove_entries(&self.dir)?;
        for entry in &self.entries {
            persist::save_entry(&self.dir, entry)?;
        }
        Ok(())
    }
}

/// Compute which entries to keep based on coverage subset relationships.
/// Runs without holding any lock — safe to call on a snapshot.
pub fn compute_eviction(snapshots: &[(u32, CoverageBitmap)]) -> Vec<bool> {
    let n = snapshots.len();
    let mut keep = vec![true; n];

    for i in 0..n {
        if !keep[i] {
            continue;
        }
        let (nz_i, ref cov_i) = snapshots[i];
        for j in (i + 1)..n {
            if !keep[j] {
                continue;
            }
            let (nz_j, ref cov_j) = snapshots[j];
            let j_sub_i = nz_j <= nz_i && cov_j.is_subset_of(cov_i);
            let i_sub_j = nz_i <= nz_j && cov_i.is_subset_of(cov_j);
            if j_sub_i && !i_sub_j {
                keep[j] = false;
            } else if i_sub_j && !j_sub_i {
                keep[i] = false;
                break;
            }
        }
    }

    keep
}

#[cfg(test)]
mod tests;
