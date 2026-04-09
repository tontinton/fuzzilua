mod cli;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use clap::Parser;
use color_eyre::eyre::{Result, bail};
use fuzzilua_cli::{
    AtomicF64, AtomicStats, CrashDb, SharedState, StatsReporter, WorkerConfig, reproduce,
    run_worker_loop, seed,
};
use fuzzilua_corpus::{Corpus, WeightedScheduler};
use fuzzilua_coverage::AtomicBitmap;
use fuzzilua_mutate::MutationEngine;
use fuzzilua_target_redis::{ENV_ALLOC_FAIL_PROB, RedisConfig, RedisTarget};
use tracing::info;

use crate::cli::Cli;

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Some(ref path) = cli.inject_lua {
        return run_inject_lua(&cli, path);
    }

    if let Some(ref path) = cli.reproduce {
        return run_reproduce(&cli, path);
    }

    if let Some(ref path) = cli.minimize_crash {
        return run_minimize_crash(&cli, path);
    }

    run_fuzz(&cli)
}

fn run_inject_lua(cli: &Cli, lua_dir: &std::path::Path) -> Result<()> {
    use fuzzilua_ir::{Instruction, Op, Program, Variable};
    use std::sync::Arc;

    fs::create_dir_all(&cli.corpus)?;
    let mut corpus = load_or_create_corpus(cli)?;
    let before = corpus.len();

    let mut lua_files: Vec<_> = fs::read_dir(lua_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "lua"))
        .collect();
    lua_files.sort();

    for path in &lua_files {
        let lua_code = fs::read_to_string(path)?;
        if lua_code.trim().is_empty() {
            continue;
        }

        let program = Program {
            instructions: vec![
                Instruction {
                    op: Op::LoadString(Arc::from(lua_code.as_str())),
                    inputs: vec![],
                    outputs: vec![Variable(0)],
                },
                Instruction {
                    op: Op::Loadstring,
                    inputs: vec![Variable(0)],
                    outputs: vec![Variable(1)],
                },
                Instruction {
                    op: Op::CallFunction {
                        arg_count: 0,
                        ret_count: 0,
                    },
                    inputs: vec![Variable(1)],
                    outputs: vec![],
                },
            ],
            next_var: 2,
        };

        corpus.add_blind(program);
        info!(file = %path.display(), "injected lua seed");
    }

    let added = corpus.len() - before;
    info!(added, total = corpus.len(), "injection complete");
    Ok(())
}

fn run_reproduce(cli: &Cli, path: &std::path::Path) -> Result<()> {
    let mut target = RedisTarget::spawn(make_redis_config(cli, false)?)?;
    let crashed = reproduce::reproduce(&mut target, path)?;
    std::process::exit(if crashed { 0 } else { 1 });
}

fn run_minimize_crash(cli: &Cli, path: &std::path::Path) -> Result<()> {
    let mut target = RedisTarget::spawn(make_redis_config(cli, false)?)?;
    let minimized = reproduce::minimize_crash(&mut target, path)?;

    let out_path = path.with_extension("minimized.bin");
    let data = bincode::serialize(&minimized)?;
    fs::write(&out_path, &data)?;
    info!(
        path = %out_path.display(),
        instructions = minimized.instructions.len(),
        "saved minimized crash"
    );
    Ok(())
}

fn run_fuzz(cli: &Cli) -> Result<()> {
    let crash_dir = cli.corpus.join("crashes");
    fs::create_dir_all(&cli.corpus)?;
    fs::create_dir_all(&crash_dir)?;

    let jobs = cli.jobs;
    info!(workers = jobs, "starting fuzzing");

    let mut seed_target = RedisTarget::spawn(make_redis_config(cli, false)?)?;
    let mut seed_corpus = load_or_create_corpus(cli)?;
    let mut seed_rng = rand::rng();

    if seed_corpus.is_empty() {
        info!("corpus is empty, seeding...");
        seed::seed_corpus(&mut seed_target, &mut seed_rng, &mut seed_corpus);
    }
    if seed_corpus.is_empty() {
        bail!("seeding produced no corpus entries; check that the target is working");
    }
    drop(seed_target);

    let shutdown = Arc::new(AtomicBool::new(false));
    install_signal_handler(Arc::clone(&shutdown));

    let shared = Arc::new(SharedState {
        corpus: RwLock::new(seed_corpus),
        coverage: AtomicBitmap::new(cli.edge_size, cli.gc_size),
        crash_db: Mutex::new(CrashDb::new()),
        stats: AtomicStats::new(),
        shutdown,
        generation_ratio: AtomicF64::new(cli.generation_ratio),
    });

    {
        let corpus = shared.corpus.read().unwrap();
        let (edge_bits, gc_bits) = corpus.total_coverage();
        info!(
            edge_bits,
            gc_bits,
            entries = corpus.len(),
            "initial corpus loaded"
        );
    }

    let engine = Arc::new(MutationEngine::new());

    let mut handles = Vec::with_capacity(jobs as usize);
    for worker_id in 0..jobs {
        let shared = Arc::clone(&shared);
        let engine = Arc::clone(&engine);
        let redis_config = make_redis_config(cli, true)?;
        let config = WorkerConfig {
            max_iters: cli.max_iters,
            crash_dir: crash_dir.clone(),
            minimize: !cli.no_minimize,
            worker_id,
        };

        handles.push(
            thread::Builder::new()
                .name(format!("worker-{worker_id}"))
                .stack_size(32 * 1024 * 1024) // 32 MB — large programs can blow 8 MB default
                .spawn(move || {
                    let mut target = match RedisTarget::spawn(redis_config) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::error!(worker = worker_id, "failed to spawn target: {e}");
                            return;
                        }
                    };

                    let mut rng = rand::rng();
                    run_worker_loop(&mut target, &shared, &engine, &config, &mut rng);
                })
                .expect("failed to spawn worker thread"),
        );
    }

    let mut reporter = StatsReporter::new(cli.stats_interval, cli.stats_json.clone(), jobs);
    let mut seed_watcher = cli
        .seed_dir
        .as_ref()
        .map(|dir| SeedWatcher::new(dir.clone()));

    // Plateau detection & periodic compaction state
    let mut last_edge_bits = 0u32;
    let mut stall_intervals = 0u32;
    let mut last_compact = std::time::Instant::now();
    let base_gen_ratio = cli.generation_ratio;
    const COMPACT_INTERVAL_SECS: u64 = 180;
    const STALL_THRESHOLD: u32 = 3; // intervals with no new edge coverage

    while !shared.shutdown.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_millis(500));

        if let Some(ref mut watcher) = seed_watcher {
            watcher.poll(&shared);
        }

        let corpus = shared.corpus.read().unwrap();
        let unique_crashes = shared.crash_db.lock().unwrap().unique_count();
        let displayed = reporter.maybe_display(&shared.stats, &corpus, unique_crashes);
        let (edge_bits, _gc_bits) = corpus.total_coverage();
        let corpus_size = corpus.len();
        drop(corpus);

        // --- Plateau detection: boost generation ratio when edge coverage stalls ---
        if displayed {
            if edge_bits > last_edge_bits {
                last_edge_bits = edge_bits;
                stall_intervals = 0;
                shared.generation_ratio.store(base_gen_ratio);
            } else {
                stall_intervals += 1;
                if stall_intervals >= STALL_THRESHOLD {
                    let boosted = (base_gen_ratio * 5.0).min(0.8);
                    shared.generation_ratio.store(boosted);
                    if stall_intervals == STALL_THRESHOLD {
                        info!(
                            boosted_ratio = boosted,
                            stall_intervals, "edge coverage stalled, boosting generation ratio"
                        );
                    }
                }
            }
        }

        // --- Periodic corpus compaction (non-blocking) ---
        // Snapshot coverage bitmaps under a read lock, compute eviction set
        // without any lock, then apply under a brief write lock.
        if last_compact.elapsed().as_secs() >= COMPACT_INTERVAL_SECS && corpus_size > 500 {
            last_compact = std::time::Instant::now();
            let shared_clone = Arc::clone(&shared);
            thread::spawn(move || {
                // Phase 1: snapshot coverage under read lock
                let snapshots: Vec<(u32, fuzzilua_coverage::CoverageBitmap)> = {
                    let corpus = shared_clone.corpus.read().unwrap();
                    corpus.snapshot_coverage()
                };

                // Phase 2: compute eviction set (no lock held)
                let keep = fuzzilua_corpus::compute_eviction(&snapshots);

                // Phase 3: apply under brief write lock
                let evictable: usize = keep.iter().filter(|&&k| !k).count();
                if evictable > 0 {
                    let mut corpus = shared_clone.corpus.write().unwrap();
                    corpus.apply_eviction(&keep);
                }
            });
        }

        if cli
            .max_iters
            .is_some_and(|max| shared.stats.total_execs() >= max)
        {
            shared.shutdown.store(true, Ordering::Relaxed);
            break;
        }
    }

    info!("waiting for workers to finish...");
    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let corpus = shared.corpus.read().unwrap();
    let (edge_bits, gc_bits) = corpus.total_coverage();
    let unique_crashes = shared.crash_db.lock().unwrap().unique_count();
    info!(
        total_execs = shared.stats.total_execs(),
        corpus_size = corpus.len(),
        edge_bits,
        gc_bits,
        total_crashes = shared.stats.crashes(),
        unique_crashes,
        "fuzzing complete"
    );

    Ok(())
}

fn make_redis_config(cli: &Cli, enable_alloc_fail: bool) -> Result<RedisConfig> {
    let binary = cli
        .redis_bin
        .clone()
        .ok_or_else(|| color_eyre::eyre::eyre!("--redis-bin is required"))?;
    let mut config = RedisConfig {
        binary,
        exec_timeout: cli.timeout,
        edge_bitmap_size: cli.edge_size,
        gc_bitmap_size: cli.gc_size,
        ..RedisConfig::default()
    }
    .with_random_port();

    if enable_alloc_fail && let Some(prob) = cli.alloc_fail_prob {
        config
            .extra_env
            .push((ENV_ALLOC_FAIL_PROB.into(), prob.to_string()));
    }

    Ok(config)
}

fn load_or_create_corpus(cli: &Cli) -> Result<Corpus> {
    let scheduler = Box::new(WeightedScheduler);
    match Corpus::load(scheduler, &cli.corpus, cli.edge_size, cli.gc_size) {
        Ok(c) => {
            if !c.is_empty() {
                info!(entries = c.len(), "loaded existing corpus");
            }
            Ok(c)
        }
        Err(e) => {
            tracing::warn!("failed to load corpus: {e}, creating new");
            Ok(Corpus::new(
                Box::new(WeightedScheduler),
                &cli.corpus,
                cli.edge_size,
                cli.gc_size,
            ))
        }
    }
}

struct SeedWatcher {
    dir: std::path::PathBuf,
    loaded_dir: std::path::PathBuf,
    last_poll: std::time::Instant,
}

impl SeedWatcher {
    fn new(dir: std::path::PathBuf) -> Self {
        let loaded_dir = dir.join(".loaded");
        Self {
            dir,
            loaded_dir,
            last_poll: std::time::Instant::now(),
        }
    }

    fn poll(&mut self, shared: &SharedState) {
        // Check every 5 seconds
        if self.last_poll.elapsed() < std::time::Duration::from_secs(5) {
            return;
        }
        self.last_poll = std::time::Instant::now();

        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut lua_files: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "lua"))
            .collect();

        if lua_files.is_empty() {
            return;
        }

        lua_files.sort();
        let _ = fs::create_dir_all(&self.loaded_dir);

        use fuzzilua_ir::{Instruction, Op, Program, Variable};
        use std::sync::Arc;

        let mut loaded = 0u32;
        for path in &lua_files {
            let lua_code = match fs::read_to_string(path) {
                Ok(s) if !s.trim().is_empty() => s,
                _ => continue,
            };

            let program = Program {
                instructions: vec![
                    Instruction {
                        op: Op::LoadString(Arc::from(lua_code.as_str())),
                        inputs: vec![],
                        outputs: vec![Variable(0)],
                    },
                    Instruction {
                        op: Op::Loadstring,
                        inputs: vec![Variable(0)],
                        outputs: vec![Variable(1)],
                    },
                    Instruction {
                        op: Op::CallFunction {
                            arg_count: 0,
                            ret_count: 0,
                        },
                        inputs: vec![Variable(1)],
                        outputs: vec![],
                    },
                ],
                next_var: 2,
            };

            {
                let mut corpus = shared.corpus.write().unwrap();
                corpus.add_blind(program);
            }
            loaded += 1;

            // Move to .loaded/ so we don't re-ingest
            if let Some(name) = path.file_name() {
                let _ = fs::rename(path, self.loaded_dir.join(name));
            }
        }

        if loaded > 0 {
            info!(loaded, "hot-loaded new lua seeds");
        }
    }
}

static SHUTDOWN: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

fn install_signal_handler(shutdown: Arc<AtomicBool>) {
    SHUTDOWN
        .set(shutdown)
        .expect("signal handler installed twice");
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGINT,
            nix::sys::signal::SigHandler::Handler(handle_signal),
        )
        .ok();
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTERM,
            nix::sys::signal::SigHandler::Handler(handle_signal),
        )
        .ok();
    }
}

extern "C" fn handle_signal(_: std::ffi::c_int) {
    if let Some(flag) = SHUTDOWN.get() {
        flag.store(true, Ordering::Relaxed);
    }
}
