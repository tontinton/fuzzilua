//! Weighted code generators and program builder.
//!
//! `CodeGenerator::generate()` returns `Option<()>`. Budget exhaustion propagates
//! via `?`, which prevents "partial emission" bugs where a generator emits half-built
//! IR and bails. Every `emit()` call returns `Option<Vec<Variable>>`, so `?` naturally
//! unwinds the whole generator on budget exhaustion.
//!
//! The `CodeGenerator` trait has only `weight()` + `generate()`. We intentionally
//! removed `required_inputs()` because it was disconnected from reality: most
//! generators returned `&[]`, and the rest used `ensure_*` helpers anyway.
//!
//! `ensure_*` helpers (ensure_table, ensure_string, etc.) create a value on demand
//! if none of the right type is in scope. They also return `Option`, so budget
//! exhaustion during value creation propagates cleanly.
//!
//! ProgramBuilder uses `BlockKind` from the IR crate directly (not a private copy).
//! `begin_block()` uses `op.opens_block()?` to validate the op is a block opener,
//! and rejects `BeginElse` (must use `begin_else()` instead, which is budget-aware).

mod builder;
mod generators;

pub use builder::ProgramBuilder;
pub use generators::{Generator, all_generators};

use fuzzilua_ir::Program;
use rand::Rng;

const MAX_RETRIES: usize = 10;

pub fn generate_program(
    rng: &mut impl Rng,
    budget: usize,
    max_depth: usize,
    generators: &[Generator],
) -> Program {
    let mut builder = ProgramBuilder::new(budget, max_depth);
    let total_weight: f64 = generators.iter().map(|g| g.weight).sum();

    while builder.remaining_budget() > 0 {
        let mut success = false;
        for _ in 0..MAX_RETRIES {
            let Some(generator) = pick_weighted(rng, generators, total_weight) else {
                break;
            };
            if (generator.generate)(&mut builder, rng).is_some() {
                success = true;
                break;
            }
        }
        if !success {
            break;
        }
    }

    builder.finish()
}

fn pick_weighted<'a>(
    rng: &mut impl Rng,
    generators: &'a [Generator],
    total_weight: f64,
) -> Option<&'a Generator> {
    if generators.is_empty() {
        return None;
    }
    let mut target = rng.random_range(0.0..total_weight);
    for g in generators {
        target -= g.weight;
        if target <= 0.0 {
            return Some(g);
        }
    }
    Some(generators.last().unwrap())
}

#[cfg(test)]
mod tests;
