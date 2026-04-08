use std::path::Path;

use color_eyre::eyre::{Result, bail};
use fuzzilua_ir::{Program, lift};
use fuzzilua_target::{ExecStatus, Target};
use tracing::info;

pub fn reproduce(target: &mut dyn Target, path: &Path) -> Result<bool> {
    let (script, _program) = load_script(path)?;

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

pub fn minimize_crash(target: &mut dyn Target, path: &Path) -> Result<Program> {
    let (_script, program) = load_script(path)?;
    let program = program.ok_or_else(|| {
        color_eyre::eyre::eyre!("--minimize-crash requires a .bin file (IR format)")
    })?;

    info!(
        path = %path.display(),
        instructions = program.instructions.len(),
        "minimizing crash reproducer"
    );

    let minimized = minimize_for_crash(&program, target);

    info!(
        before = program.instructions.len(),
        after = minimized.instructions.len(),
        "crash minimization complete"
    );

    Ok(minimized)
}

fn minimize_for_crash(program: &Program, target: &mut dyn Target) -> Program {
    let mut best = program.clone();

    // Try removing each instruction and see if the crash still reproduces
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < best.instructions.len() {
            if best.instructions.len() <= 1 {
                break;
            }
            let mut candidate = best.clone();
            candidate.instructions.remove(i);

            let script = lift(&candidate);
            let crashes = match target.execute(&script) {
                Ok(r) => matches!(r.status, ExecStatus::Crash(_)),
                Err(_) => {
                    let _ = target.restart();
                    false
                }
            };
            let _ = target.restart();

            if crashes {
                best = candidate;
                changed = true;
            } else {
                i += 1;
            }
        }
    }

    best
}

fn load_script(path: &Path) -> Result<(String, Option<Program>)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "lua" => {
            let script = std::fs::read_to_string(path)?;
            Ok((script, None))
        }
        "bin" => {
            let data = std::fs::read(path)?;
            let program: Program = bincode::deserialize(&data)?;
            let script = lift(&program);
            Ok((script, Some(program)))
        }
        _ => bail!("unsupported file extension: {ext:?} (expected .lua or .bin)"),
    }
}
