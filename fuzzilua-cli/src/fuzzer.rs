use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use fuzzilua_corpus::{Corpus, minimize};
use fuzzilua_coverage::AtomicBitmap;
use fuzzilua_ir::lift;
use fuzzilua_mutate::{HybridEngine, MutationEngine};
use fuzzilua_target::{ExecStatus, Target};
use rand::Rng;
use tracing::{debug, error, warn};

use crate::crash::{CrashDb, build_crash_report, crash_hash};
use crate::stats::AtomicStats;

/// Atomic f64 via bit-cast to u64.
pub struct AtomicF64(AtomicU64);

impl AtomicF64 {
    pub fn new(val: f64) -> Self {
        Self(AtomicU64::new(val.to_bits()))
    }
    pub fn load(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn store(&self, val: f64) {
        self.0.store(val.to_bits(), Ordering::Relaxed);
    }
}

pub struct SharedState {
    pub corpus: RwLock<Corpus>,
    pub coverage: AtomicBitmap,
    pub crash_db: Mutex<CrashDb>,
    pub stats: AtomicStats,
    pub shutdown: Arc<AtomicBool>,
    pub generation_ratio: AtomicF64,
}

pub struct WorkerConfig {
    pub max_iters: Option<u64>,
    pub crash_dir: std::path::PathBuf,
    pub minimize: bool,
    pub worker_id: u32,
}

pub fn run_worker_loop(
    target: &mut dyn Target,
    shared: &SharedState,
    engine: &MutationEngine,
    config: &WorkerConfig,
    rng: &mut impl Rng,
) {
    let hybrid = HybridEngine::new();

    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            debug!(worker = config.worker_id, "shutdown requested");
            break;
        }
        if let Some(max) = config.max_iters
            && shared.stats.total_execs() >= max
        {
            debug!(worker = config.worker_id, "global max iterations reached");
            break;
        }

        let gen_ratio = shared.generation_ratio.load();
        let program = if rng.random_range(0.0..1.0) < gen_ratio {
            hybrid.generate(rng)
        } else {
            let corpus = shared.corpus.read().unwrap();
            if corpus.is_empty() {
                drop(corpus);
                hybrid.generate(rng)
            } else {
                let mut p = corpus.select(rng).program.clone();
                drop(corpus);
                engine.mutate(&mut p, rng);
                p
            }
        };

        // Cap program size to prevent unbounded growth through mutation.
        // Very large programs slow execution and can blow the stack.
        const MAX_INSTRUCTIONS: usize = 512;
        if program.instructions.len() > MAX_INSTRUCTIONS {
            shared.stats.record_exec();
            continue;
        }

        let script = lift(&program);

        let result = match target.execute(&script) {
            Ok(r) => r,
            Err(e) => {
                warn!(worker = config.worker_id, "target execution error: {e}");
                let _ = target.restart();
                shared.stats.record_exec();
                continue;
            }
        };

        shared.stats.record_exec();

        let needs_restart = match &result.status {
            ExecStatus::Crash(crash_info) => {
                shared.stats.record_crash();
                handle_crash(shared, &config.crash_dir, &program, crash_info);
                true
            }
            ExecStatus::Ok | ExecStatus::RuntimeError(_) => {
                let coverage = target.collect_coverage();

                // Strict merge: only 0→nonzero byte transitions count as
                // novel. Prevents corpus bloat from hit-count bucket noise.
                if shared.coverage.merge_if_new_edge_strict(&coverage) {
                    let (program, coverage) = if config.minimize {
                        let min_prog = minimize(&program, target);
                        let script = lift(&min_prog);
                        if target.execute(&script).is_ok() {
                            let min_cov = target.collect_coverage();
                            let _ = target.reset();
                            (min_prog, min_cov)
                        } else {
                            let _ = target.reset();
                            (program, coverage)
                        }
                    } else {
                        (program, coverage)
                    };
                    let mut corpus = shared.corpus.write().unwrap();
                    corpus.add_unchecked(program, coverage);
                    debug!(
                        worker = config.worker_id,
                        corpus_size = corpus.len(),
                        "new edge coverage found"
                    );
                }
                false
            }
            ExecStatus::Timeout => {
                debug!(worker = config.worker_id, "execution timed out");
                false
            }
            ExecStatus::ConnectionLost => {
                warn!(worker = config.worker_id, "connection lost, restarting");
                true
            }
        };

        if needs_restart {
            let _ = target.restart();
        } else {
            let _ = target.reset();
        }
    }

    debug!(worker = config.worker_id, "worker finished");
}

fn handle_crash(
    shared: &SharedState,
    crash_dir: &Path,
    program: &fuzzilua_ir::Program,
    crash_info: &fuzzilua_target::CrashInfo,
) {
    let is_new = {
        let mut db = shared.crash_db.lock().unwrap();
        db.add(crash_info, program)
    };

    if is_new {
        let report = build_crash_report(crash_info);
        let signal = crash_info.signal.unwrap_or(0);
        error!(
            signal,
            hash = crash_hash(crash_info),
            summary = %report.lines().next().unwrap_or(""),
            "NEW UNIQUE CRASH"
        );
        if let Err(e) = fuzzilua_corpus::save_crash(crash_dir, program, signal, &report) {
            error!("failed to save crash: {e}");
        }
    }
}
