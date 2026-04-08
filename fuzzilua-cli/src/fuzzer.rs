use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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

pub struct SharedState {
    pub corpus: RwLock<Corpus>,
    pub coverage: AtomicBitmap,
    pub crash_db: Mutex<CrashDb>,
    pub stats: AtomicStats,
    pub shutdown: Arc<AtomicBool>,
}

pub struct WorkerConfig {
    pub max_iters: Option<u64>,
    pub crash_dir: std::path::PathBuf,
    pub minimize: bool,
    pub generation_ratio: f64,
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

        let mut program = {
            let corpus = shared.corpus.read().unwrap();
            if !corpus.is_empty() && rng.random_range(0.0..1.0) >= config.generation_ratio {
                let entry = corpus.select(rng);
                let mut p = entry.program.clone();
                engine.mutate(&mut p, rng);
                p
            } else {
                hybrid.generate(rng)
            }
        };

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
                let mut coverage = target.collect_coverage();
                coverage.classify_counts();

                if shared.coverage.has_new_bits(&coverage) {
                    if config.minimize {
                        program = minimize(&program, target);
                        let script = lift(&program);
                        if target.execute(&script).is_ok() {
                            coverage = target.collect_coverage();
                            coverage.classify_counts();
                        }
                        let _ = target.reset();
                    }
                    if shared.coverage.merge_if_new(&coverage) {
                        let mut corpus = shared.corpus.write().unwrap();
                        corpus.add_unchecked(program, coverage);
                        debug!(
                            worker = config.worker_id,
                            corpus_size = corpus.len(),
                            "new coverage found"
                        );
                    }
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
