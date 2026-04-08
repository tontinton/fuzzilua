use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "fuzzilua", about = "Fuzzilli-inspired fuzzer for Lua VMs")]
pub struct Cli {
    #[arg(long, help = "Path to redis-server binary")]
    pub redis_bin: Option<PathBuf>,

    #[arg(long, default_value_t = default_jobs(), help = "Number of fuzzer workers (default: available parallelism)")]
    pub jobs: u32,

    #[arg(long, default_value = "corpus", help = "Corpus directory")]
    pub corpus: PathBuf,

    #[arg(long, default_value = "5s", value_parser = parse_duration, help = "Execution timeout")]
    pub timeout: Duration,

    #[arg(long, help = "Maximum iterations (runs forever if unset)")]
    pub max_iters: Option<u64>,

    #[arg(long, help = "Reproduce a crash from .lua or .bin file")]
    pub reproduce: Option<PathBuf>,

    #[arg(long, help = "Minimize a crash reproducer (.bin file)")]
    pub minimize_crash: Option<PathBuf>,

    #[arg(long, default_value = "65536", help = "Edge bitmap size")]
    pub edge_size: usize,

    #[arg(long, default_value = "65536", help = "GC bitmap size")]
    pub gc_size: usize,

    #[arg(long, help = "Disable minimization of new corpus entries")]
    pub no_minimize: bool,

    #[arg(
        long,
        default_value = "0.3",
        value_parser = parse_ratio,
        help = "Probability that a new program is generated from scratch vs mutated from corpus (0.0-1.0)"
    )]
    pub generation_ratio: f64,

    #[arg(long, help = "Append JSON stats per interval to this file")]
    pub stats_json: Option<PathBuf>,

    #[arg(long, default_value = "5s", value_parser = parse_duration, help = "Stats display interval")]
    pub stats_interval: Duration,

    #[arg(long, help = "Enable verbose per-execution logging")]
    pub verbose: bool,

    #[arg(
        long,
        value_parser = parse_ratio,
        help = "Probability of allocation failure injection (0.0-1.0)"
    )]
    pub alloc_fail_prob: Option<f64>,
}

fn default_jobs() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn parse_ratio(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|e: std::num::ParseFloatError| e.to_string())?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{v} is not in range 0.0..=1.0"))
    }
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    if let Some(secs) = s.strip_suffix('s') {
        secs.parse::<f64>()
            .map(Duration::from_secs_f64)
            .map_err(|e| e.to_string())
    } else if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|e| e.to_string())
    } else {
        s.parse::<f64>()
            .map(Duration::from_secs_f64)
            .map_err(|_| format!("invalid duration: {s} (use e.g. 5s or 500ms)"))
    }
}
