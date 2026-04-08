use fuzzilua_coverage::CoverageBitmap;
use fuzzilua_ir::{Instruction, Op, Program, VarBitset, lift};
use fuzzilua_target::{ExecStatus, Target};
use tracing::debug;

const MIN_PROGRAM_SIZE: usize = 5;
const DETERMINISM_RUNS: usize = 3;

pub fn minimize(program: &Program, target: &mut dyn Target) -> Program {
    if program.instructions.len() < MIN_PROGRAM_SIZE {
        return program.clone();
    }

    let Some(ref_coverage) = stable_coverage(program, target) else {
        return program.clone();
    };

    let mut current = program.clone();
    const MAX_PASSES: usize = 10;
    let mut changed = true;
    let mut pass = 0;

    while changed && pass < MAX_PASSES {
        changed = false;
        pass += 1;
        changed |= nop_removal(&mut current);
        changed |= gc_consolidation(&mut current);
        changed |= dead_variable_elimination(&mut current);
        changed |= instruction_removal(&mut current, target, &ref_coverage);
    }

    debug!(
        before = program.instructions.len(),
        after = current.instructions.len(),
        "minimization complete"
    );
    current
}

fn stable_coverage(program: &Program, target: &mut dyn Target) -> Option<CoverageBitmap> {
    let script = lift(program);
    let mut result: Option<CoverageBitmap> = None;

    for _ in 0..DETERMINISM_RUNS {
        let exec = target.execute(&script).ok()?;
        match exec.status {
            ExecStatus::Ok | ExecStatus::RuntimeError(_) => {}
            _ => return None,
        }
        let cov = target.collect_coverage();
        let _ = target.reset();

        result = Some(match result {
            None => cov,
            Some(prev) => intersect_coverage(&prev, &cov),
        });
    }

    result
}

fn intersect_coverage(a: &CoverageBitmap, b: &CoverageBitmap) -> CoverageBitmap {
    debug_assert_eq!(a.edge_len(), b.edge_len(), "coverage edge size mismatch");
    debug_assert_eq!(a.gc_len(), b.gc_len(), "coverage gc size mismatch");
    let mut result = CoverageBitmap::new(a.edge_len(), a.gc_len());
    for (r, (&x, &y)) in result
        .as_bytes_mut()
        .iter_mut()
        .zip(a.as_bytes().iter().zip(b.as_bytes()))
    {
        *r = x & y;
    }
    result
}

fn coverage_preserved(
    program: &Program,
    target: &mut dyn Target,
    ref_coverage: &CoverageBitmap,
) -> bool {
    let script = lift(program);
    let Ok(exec) = target.execute(&script) else {
        let _ = target.restart();
        return false;
    };
    match exec.status {
        ExecStatus::Ok | ExecStatus::RuntimeError(_) => {}
        ExecStatus::Crash(_) | ExecStatus::ConnectionLost => {
            let _ = target.restart();
            return false;
        }
        ExecStatus::Timeout => {
            let _ = target.reset();
            return false;
        }
    }
    let cov = target.collect_coverage();
    let _ = target.reset();
    ref_coverage.is_subset_of(&cov)
}

fn nop_removal(program: &mut Program) -> bool {
    let before = program.instructions.len();
    program.instructions.retain(|instr| instr.op != Op::Nop);
    program.instructions.len() < before
}

fn gc_consolidation(program: &mut Program) -> bool {
    let before = program.instructions.len();
    let mut prev_was_gc = false;
    program.instructions.retain(|instr| {
        let is_gc = matches!(instr.op, Op::CollectGarbage(_));
        let keep = !(is_gc && prev_was_gc);
        prev_was_gc = is_gc;
        keep
    });
    program.instructions.len() < before
}

fn dead_variable_elimination(program: &mut Program) -> bool {
    let mut ever_changed = false;
    loop {
        let mut used = VarBitset::with_capacity(program.next_var);
        for instr in &program.instructions {
            for &v in &instr.inputs {
                used.insert(v);
            }
        }

        let mut changed = false;
        for instr in &mut program.instructions {
            if instr.op.is_effectful() {
                continue;
            }
            if instr.outputs.is_empty() {
                continue;
            }
            let all_dead = instr.outputs.iter().all(|v| !used.contains(v));
            if all_dead {
                *instr = Instruction::nop();
                changed = true;
            }
        }
        if !changed {
            break;
        }
        ever_changed = true;
    }
    ever_changed
}

fn instruction_removal(
    program: &mut Program,
    target: &mut dyn Target,
    ref_coverage: &CoverageBitmap,
) -> bool {
    let mut changed = false;

    for i in (0..program.instructions.len()).rev() {
        let op = &program.instructions[i].op;
        if op.opens_block().is_some() || op.closes_block().is_some() {
            continue;
        }
        if matches!(op, Op::Break | Op::Return | Op::Nop) {
            continue;
        }

        let saved = program.instructions[i].clone();
        program.instructions[i] = Instruction::nop();

        if program.validate().is_ok() && coverage_preserved(program, target, ref_coverage) {
            changed = true;
        } else {
            program.instructions[i] = saved;
        }
    }

    changed
}
