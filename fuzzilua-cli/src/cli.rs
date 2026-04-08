use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "fuzzilua", about = "Fuzzilli-inspired fuzzer for Lua VMs")]
pub struct Cli {
    #[arg(long, help = "Path to redis-server binary")]
    pub redis_bin: Option<PathBuf>,

    #[arg(long, default_value = "1", help = "Number of fuzzer workers")]
    pub jobs: u32,

    #[arg(long, default_value = "corpus", help = "Corpus directory")]
    pub corpus: PathBuf,

    #[arg(long, default_value = "5s", value_parser = parse_duration, help = "Execution timeout")]
    pub timeout: Duration,

    #[arg(long, help = "Maximum iterations (runs forever if unset)")]
    pub max_iters: Option<u64>,

    #[arg(long, help = "Reproduce a crash from .lua or .bin file")]
    pub reproduce: Option<PathBuf>,

    #[arg(long, default_value = "65536", help = "Edge bitmap size")]
    pub edge_size: usize,

    #[arg(long, default_value = "65536", help = "GC bitmap size")]
    pub gc_size: usize,
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
