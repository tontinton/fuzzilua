use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fuzzilua_corpus::Corpus;

#[derive(Default)]
pub struct AtomicStats {
    total_execs: AtomicU64,
    crashes: AtomicU64,
}

impl AtomicStats {
    pub fn new() -> Self {
        Self {
            total_execs: AtomicU64::new(0),
            crashes: AtomicU64::new(0),
        }
    }

    pub fn record_exec(&self) {
        self.total_execs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_crash(&self) {
        self.crashes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn total_execs(&self) -> u64 {
        self.total_execs.load(Ordering::Relaxed)
    }

    pub fn crashes(&self) -> u64 {
        self.crashes.load(Ordering::Relaxed)
    }
}

pub struct StatsReporter {
    start_time: Instant,
    last_display: Instant,
    last_execs: u64,
    last_execs_time: Instant,
    display_interval: Duration,
    json_path: Option<PathBuf>,
    workers: u32,
}

impl StatsReporter {
    pub fn new(display_interval: Duration, json_path: Option<PathBuf>, workers: u32) -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            last_display: now,
            last_execs: 0,
            last_execs_time: now,
            display_interval,
            json_path,
            workers,
        }
    }

    pub fn maybe_display(&mut self, stats: &AtomicStats, corpus: &Corpus, unique_crashes: usize) {
        let now = Instant::now();
        if now.duration_since(self.last_display) < self.display_interval {
            return;
        }
        self.last_display = now;
        self.display(stats, corpus, unique_crashes);
    }

    pub fn display(&mut self, stats: &AtomicStats, corpus: &Corpus, unique_crashes: usize) {
        let elapsed = self.start_time.elapsed();
        let total = stats.total_execs();
        let (edge_bits, gc_bits) = corpus.total_coverage();
        let (edge_total, gc_total) = corpus.bitmap_sizes();

        let now = Instant::now();
        let dt = now.duration_since(self.last_execs_time).as_secs_f64();
        let execs_per_sec = if dt > 0.0 {
            (total - self.last_execs) as f64 / dt
        } else {
            0.0
        };
        self.last_execs = total;
        self.last_execs_time = now;

        let snap = StatsSnapshot {
            elapsed,
            total_execs: total,
            execs_per_sec,
            corpus_size: corpus.len(),
            edge_bits,
            edge_total,
            gc_bits,
            gc_total,
            unique_crashes,
            workers: self.workers,
        };

        let line = format_stats_line(&snap);
        let _ = writeln!(std::io::stderr(), "{line}");

        if let Some(ref json_path) = self.json_path {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let json = format!(
                r#"{{"timestamp":{ts},"elapsed_secs":{:.1},"total_execs":{total},"execs_per_sec":{execs_per_sec:.1},"corpus_size":{},"edge_bits":{edge_bits},"edge_total":{edge_total},"gc_bits":{gc_bits},"gc_total":{gc_total},"crashes":{},"unique_crashes":{unique_crashes},"workers":{}}}"#,
                elapsed.as_secs_f64(),
                corpus.len(),
                stats.crashes(),
                self.workers,
            );
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(json_path)
            {
                let _ = writeln!(f, "{json}");
            }
        }
    }
}

pub struct StatsSnapshot {
    pub elapsed: Duration,
    pub total_execs: u64,
    pub execs_per_sec: f64,
    pub corpus_size: usize,
    pub edge_bits: u32,
    pub edge_total: usize,
    pub gc_bits: u32,
    pub gc_total: usize,
    pub unique_crashes: usize,
    pub workers: u32,
}

pub fn format_stats_line(s: &StatsSnapshot) -> String {
    let hh = s.elapsed.as_secs() / 3600;
    let mm = (s.elapsed.as_secs() % 3600) / 60;
    let ss = s.elapsed.as_secs() % 60;
    format!(
        "[{hh:02}:{mm:02}:{ss:02}] execs: {} ({:.0}/sec) | corpus: {} | edge: {}/{} ({:.1}%) | gc: {}/{} ({:.1}%) | crashes: {} | workers: {}",
        s.total_execs,
        s.execs_per_sec,
        s.corpus_size,
        s.edge_bits,
        s.edge_total,
        saturation_pct(s.edge_bits, s.edge_total),
        s.gc_bits,
        s.gc_total,
        saturation_pct(s.gc_bits, s.gc_total),
        s.unique_crashes,
        s.workers,
    )
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
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn format_stats_line_contains_all_fields() {
        let line = format_stats_line(&StatsSnapshot {
            elapsed: Duration::from_secs(323),
            total_execs: 42000,
            execs_per_sec: 8400.0,
            corpus_size: 312,
            edge_bits: 14023,
            edge_total: 65536,
            gc_bits: 892,
            gc_total: 65536,
            unique_crashes: 3,
            workers: 4,
        });
        for expected in ["42000", "8400", "312", "14023", "892", "3", "4"] {
            assert!(line.contains(expected), "missing {expected:?} in: {line}");
        }
    }

    #[test]
    fn json_output_is_valid() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let atomic = Arc::new(AtomicStats::new());
        for _ in 0..100 {
            atomic.record_exec();
        }
        atomic.record_crash();

        let corpus = fuzzilua_corpus::Corpus::new(
            Box::new(fuzzilua_corpus::WeightedScheduler),
            tmp.path().parent().unwrap(),
            64,
            64,
        );

        let mut reporter = StatsReporter::new(Duration::from_millis(0), Some(path.clone()), 4);
        reporter.display(&atomic, &corpus, 1);

        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed["total_execs"].as_u64().unwrap() >= 100);
            assert_eq!(parsed["workers"].as_u64().unwrap(), 4);
        }
    }
}
