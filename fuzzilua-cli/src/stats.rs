use std::collections::VecDeque;
use std::io::Write;
use std::time::{Duration, Instant};

use fuzzilua_corpus::Corpus;
use fuzzilua_target::ExecStatus;

const ROLLING_WINDOW: Duration = Duration::from_secs(10);
const DISPLAY_INTERVAL: Duration = Duration::from_secs(5);

pub struct Stats {
    total_execs: u64,
    crashes: u64,
    exec_timestamps: VecDeque<Instant>,
    last_display: Instant,
    start_time: Instant,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            total_execs: 0,
            crashes: 0,
            exec_timestamps: VecDeque::new(),
            last_display: now,
            start_time: now,
        }
    }

    pub fn record(&mut self, status: &ExecStatus) {
        self.total_execs += 1;
        let now = Instant::now();
        self.exec_timestamps.push_back(now);
        self.prune_old_timestamps(now);

        if matches!(status, ExecStatus::Crash(_)) {
            self.crashes += 1;
        }
    }

    pub fn total_execs(&self) -> u64 {
        self.total_execs
    }

    pub fn crashes(&self) -> u64 {
        self.crashes
    }

    pub fn execs_per_sec(&self) -> f64 {
        let len = self.exec_timestamps.len();
        if len <= 1 {
            return len as f64;
        }
        let oldest = self.exec_timestamps[0];
        let elapsed = Instant::now().duration_since(oldest);
        if elapsed.is_zero() {
            return len as f64;
        }
        len as f64 / elapsed.as_secs_f64()
    }

    pub fn maybe_display(&mut self, corpus: &Corpus, program_size: usize) {
        let now = Instant::now();
        if now.duration_since(self.last_display) < DISPLAY_INTERVAL {
            return;
        }
        self.last_display = now;
        self.display(corpus, program_size);
    }

    fn display(&self, corpus: &Corpus, program_size: usize) {
        let elapsed = self.start_time.elapsed();
        let (edge_bits, gc_bits) = corpus.total_coverage();
        let (edge_total, gc_total) = corpus.bitmap_sizes();

        let _ = writeln!(
            std::io::stderr(),
            "[{:>6.0}s] execs: {} ({:.0}/s) | corpus: {} | edges: {} ({:.1}%) | gc: {} ({:.1}%) | crashes: {} | prog_size: {}",
            elapsed.as_secs_f64(),
            self.total_execs,
            self.execs_per_sec(),
            corpus.len(),
            edge_bits,
            saturation_pct(edge_bits, edge_total),
            gc_bits,
            saturation_pct(gc_bits, gc_total),
            self.crashes,
            program_size,
        );
    }

    fn prune_old_timestamps(&mut self, now: Instant) {
        let cutoff = now - ROLLING_WINDOW;
        while self.exec_timestamps.front().is_some_and(|t| *t < cutoff) {
            self.exec_timestamps.pop_front();
        }
    }
}

fn saturation_pct(bits: u32, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        bits as f64 / total as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuzzilua_target::ExecStatus;

    fn crash_status() -> ExecStatus {
        ExecStatus::Crash(fuzzilua_target::CrashInfo {
            signal: Some(11),
            asan_report: None,
            ubsan_report: None,
            script: String::new(),
        })
    }

    #[test]
    fn record_tracks_execs_and_crashes() {
        let mut stats = Stats::new();
        stats.record(&ExecStatus::Ok);
        stats.record(&crash_status());
        stats.record(&ExecStatus::Ok);
        assert_eq!(stats.total_execs(), 3);
        assert_eq!(stats.crashes(), 1);
    }

    #[test]
    fn execs_per_sec_zero_when_empty() {
        let stats = Stats::new();
        assert_eq!(stats.execs_per_sec(), 0.0);
    }

    #[test]
    fn execs_per_sec_positive_after_records() {
        let mut stats = Stats::new();
        for _ in 0..100 {
            stats.record(&ExecStatus::Ok);
        }
        assert!(stats.execs_per_sec() > 0.0);
    }
}
