use crate::CorpusEntry;
use rand::RngCore;

pub trait CorpusScheduler: Send + Sync {
    fn select(&self, entries: &[CorpusEntry], rng: &mut dyn RngCore) -> usize;
}

pub struct UniformScheduler;

impl CorpusScheduler for UniformScheduler {
    fn select(&self, entries: &[CorpusEntry], rng: &mut dyn RngCore) -> usize {
        bounded_rand(rng, entries.len())
    }
}

pub struct WeightedScheduler;

impl CorpusScheduler for WeightedScheduler {
    fn select(&self, entries: &[CorpusEntry], rng: &mut dyn RngCore) -> usize {
        let n = entries.len().max(1) as f64;
        let weights: Vec<f64> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let unique = e.cached_nonzero as f64;
                // Recency bias: newer entries (higher index) get up to 2x weight.
                // This favors less-explored entries since mutation_count is not tracked at runtime.
                let recency = 1.0 + (i as f64 / n);
                unique * recency / (1.0 + e.mutation_count as f64)
            })
            .collect();
        weighted_select(&weights, rng)
    }
}

pub struct FocusedScheduler;

impl CorpusScheduler for FocusedScheduler {
    fn select(&self, entries: &[CorpusEntry], rng: &mut dyn RngCore) -> usize {
        let weights: Vec<f64> = entries
            .iter()
            .map(|e| {
                let (_, gc_bits) = e.coverage.count_bits();
                (gc_bits as f64).max(1.0)
            })
            .collect();
        weighted_select(&weights, rng)
    }
}

fn weighted_select(weights: &[f64], rng: &mut dyn RngCore) -> usize {
    debug_assert!(!weights.is_empty());
    let total: f64 = weights.iter().sum();
    if total == 0.0 {
        return bounded_rand(rng, weights.len());
    }
    let threshold = (rng.next_u64() as f64 / u64::MAX as f64) * total;
    let mut cumulative = 0.0;
    for (i, &w) in weights.iter().enumerate() {
        cumulative += w;
        if cumulative >= threshold {
            return i;
        }
    }
    weights.len() - 1
}

/// Unbiased `[0, len)` via rejection sampling.
fn bounded_rand(rng: &mut dyn RngCore, len: usize) -> usize {
    debug_assert!(len > 0);
    let len = len as u64;
    let threshold = u64::MAX - (u64::MAX % len);
    loop {
        let r = rng.next_u64();
        if r < threshold {
            return (r % len) as usize;
        }
    }
}
