mod cli;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use clap::Parser;
use color_eyre::eyre::{Result, bail};
use fuzzilua_cli::{
    AtomicStats, CrashDb, SharedState, StatsReporter, WorkerConfig, reproduce, run_worker_loop,
    seed,
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

    if let Some(ref path) = cli.reproduce {
        return run_reproduce(&cli, path);
    }

    if let Some(ref path) = cli.minimize_crash {
        return run_minimize_crash(&cli, path);
    }

    run_fuzz(&cli)
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
            generation_ratio: cli.generation_ratio,
            worker_id,
        };

        handles.push(
            thread::Builder::new()
                .name(format!("worker-{worker_id}"))
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

    while !shared.shutdown.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_millis(500));

        let corpus = shared.corpus.read().unwrap();
        let unique_crashes = shared.crash_db.lock().unwrap().unique_count();
        reporter.maybe_display(&shared.stats, &corpus, unique_crashes);

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

    if enable_alloc_fail
        && let Some(prob) = cli.alloc_fail_prob
    {
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
