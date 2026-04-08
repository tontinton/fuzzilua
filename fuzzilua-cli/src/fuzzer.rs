use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fuzzilua_corpus::{Corpus, minimize};
use fuzzilua_ir::lift;
use fuzzilua_mutate::{HybridEngine, MutationEngine};
use fuzzilua_target::{ExecStatus, Target};
use rand::Rng;
use tracing::{debug, error, info, warn};

use crate::crash::build_crash_report;
use crate::stats::Stats;

pub struct FuzzerConfig {
    pub max_iters: Option<u64>,
    pub crash_dir: std::path::PathBuf,
    pub minimize: bool,
    pub generation_ratio: f64,
}

pub fn run_fuzzer_loop(
    target: &mut dyn Target,
    corpus: &mut Corpus,
    engine: &MutationEngine,
    config: &FuzzerConfig,
    shutdown: Arc<AtomicBool>,
    rng: &mut impl Rng,
) -> Stats {
    let hybrid = HybridEngine::new();
    let mut stats = Stats::new();
    let mut iter: u64 = 0;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            info!("shutdown requested, exiting fuzzer loop");
            break;
        }
        if config.max_iters.is_some_and(|max| iter >= max) {
            info!(total = iter, "reached max iterations");
            break;
        }

        let mut program =
            if !corpus.is_empty() && rng.random_range(0.0..1.0) >= config.generation_ratio {
                let entry = corpus.select(rng);
                let mut p = entry.program.clone();
                engine.mutate(&mut p, rng);
                p
            } else {
                hybrid.generate(rng)
            };
        let script = lift(&program);
        let program_size = program.instructions.len();

        let result = match target.execute(&script) {
            Ok(r) => r,
            Err(e) => {
                warn!("target execution error: {e}");
                let _ = target.restart();
                iter += 1;
                stats.record(&ExecStatus::ConnectionLost);
                continue;
            }
        };

        stats.record(&result.status);
        iter += 1;

        let needs_restart = match &result.status {
            ExecStatus::Crash(crash_info) => {
                handle_crash(&config.crash_dir, &program, crash_info);
                true
            }
            ExecStatus::Ok | ExecStatus::RuntimeError(_) => {
                let mut coverage = target.collect_coverage();
                coverage.classify_counts();
                if config.minimize {
                    program = minimize(&program, target);
                    let script = lift(&program);
                    if target.execute(&script).is_ok() {
                        coverage = target.collect_coverage();
                        coverage.classify_counts();
                    }
                    let _ = target.reset();
                }
                if corpus.add(program, coverage) {
                    debug!(corpus_size = corpus.len(), "new coverage found");
                }
                false
            }
            ExecStatus::Timeout => {
                debug!("execution timed out, discarding");
                false
            }
            ExecStatus::ConnectionLost => {
                warn!("connection lost, restarting target");
                true
            }
        };

        if needs_restart {
            let _ = target.restart();
        } else {
            let _ = target.reset();
        }

        stats.maybe_display(corpus, program_size);
    }

    stats
}

fn handle_crash(
    crash_dir: &Path,
    program: &fuzzilua_ir::Program,
    crash_info: &fuzzilua_target::CrashInfo,
) {
    let report = build_crash_report(crash_info);
    let signal = crash_info.signal.unwrap_or(0);

    error!(
        signal,
        summary = %report.lines().next().unwrap_or(""),
        "CRASH DETECTED"
    );

    if let Err(e) = fuzzilua_corpus::save_crash(crash_dir, program, signal, &report) {
        error!("failed to save crash: {e}");
    }
}
