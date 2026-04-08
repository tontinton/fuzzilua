mod cli;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use color_eyre::eyre::{Result, bail};
use fuzzilua_corpus::{Corpus, WeightedScheduler};
use fuzzilua_mutate::MutationEngine;
use fuzzilua_target_redis::{RedisConfig, RedisTarget};
use tracing::info;

use crate::cli::Cli;
use fuzzilua_cli::{FuzzerConfig, reproduce, seed};

fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    if let Some(ref path) = cli.reproduce {
        return run_reproduce(&cli, path);
    }

    run_fuzz(&cli)
}

fn run_reproduce(cli: &Cli, path: &std::path::Path) -> Result<()> {
    let mut target = create_target(cli)?;
    let crashed = reproduce::reproduce(&mut target, path)?;
    std::process::exit(if crashed { 0 } else { 1 });
}

fn run_fuzz(cli: &Cli) -> Result<()> {
    let crash_dir = cli.corpus.join("crashes");
    fs::create_dir_all(&cli.corpus)?;
    fs::create_dir_all(&crash_dir)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    install_signal_handler(Arc::clone(&shutdown));

    let mut target = create_target(cli)?;
    let engine = MutationEngine::new();

    let mut corpus = load_or_create_corpus(cli)?;
    let mut rng = rand::rng();

    if corpus.is_empty() {
        info!("corpus is empty, seeding...");
        seed::seed_corpus(&mut target, &mut rng, &mut corpus);
    }

    if corpus.is_empty() {
        bail!("seeding produced no corpus entries; check that the target is working");
    }

    let config = FuzzerConfig {
        max_iters: cli.max_iters,
        crash_dir,
    };

    let final_stats = fuzzilua_cli::run_fuzzer_loop(
        &mut target,
        &mut corpus,
        &engine,
        &config,
        Arc::clone(&shutdown),
        &mut rng,
    );

    let (edge_bits, gc_bits) = corpus.total_coverage();
    info!(
        total_execs = final_stats.total_execs(),
        corpus_size = corpus.len(),
        edge_bits,
        gc_bits,
        crashes = final_stats.crashes(),
        "fuzzing complete"
    );

    Ok(())
}

fn create_target(cli: &Cli) -> Result<RedisTarget> {
    let binary = cli
        .redis_bin
        .clone()
        .ok_or_else(|| color_eyre::eyre::eyre!("--redis-bin is required"))?;
    let config = RedisConfig {
        binary,
        exec_timeout: cli.timeout,
        edge_bitmap_size: cli.edge_size,
        gc_bitmap_size: cli.gc_size,
        ..RedisConfig::default()
    }
    .with_random_port();
    Ok(RedisTarget::spawn(config)?)
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
