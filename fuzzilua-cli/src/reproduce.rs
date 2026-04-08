use std::path::Path;

use color_eyre::eyre::{Result, bail};
use fuzzilua_ir::{Program, lift};
use fuzzilua_target::{ExecStatus, Target};
use tracing::info;

pub fn reproduce(target: &mut dyn Target, path: &Path) -> Result<bool> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let script = match ext {
        "lua" => std::fs::read_to_string(path)?,
        "bin" => {
            let data = std::fs::read(path)?;
            let program: Program = bincode::deserialize(&data)?;
            lift(&program)
        }
        _ => bail!("unsupported file extension: {ext:?} (expected .lua or .bin)"),
    };

    info!(path = %path.display(), "reproducing script ({} bytes)", script.len());

    let result = target.execute(&script)?;

    match &result.status {
        ExecStatus::Crash(info) => {
            info!(signal = info.signal, "crash reproduced");
            if !result.stderr.is_empty() {
                eprintln!("--- stderr ---\n{}", result.stderr);
            }
            Ok(true)
        }
        status => {
            info!(?status, "no crash");
            if !result.stderr.is_empty() {
                eprintln!("--- stderr ---\n{}", result.stderr);
            }
            Ok(false)
        }
    }
}
