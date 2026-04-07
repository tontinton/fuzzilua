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
