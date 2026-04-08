//! Weighted code generators and program builder.
//!
//! Each `Generator` has a `weight` and a `generate` fn that returns `Option<()>`.
//! Budget exhaustion propagates via `?` through `emit()` and `ensure_*` helpers,
//! so a generator never emits half-built IR.

mod builder;
mod generators;
mod templates;

pub use builder::ProgramBuilder;
pub use generators::{Generator, all_generators};
pub use templates::{ProgramTemplate, all_templates};

use fuzzilua_ir::Program;
use rand::Rng;
use rand::RngCore;

const MAX_RETRIES: usize = 10;

pub fn generate_program(
    rng: &mut dyn RngCore,
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

pub(crate) fn pick_weighted<'a>(
    rng: &mut dyn RngCore,
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
