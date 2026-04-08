use fuzzilua_ir::Program;
use rand::Rng;
use tracing::trace;

use crate::mutators::{
    CallbackGcMutator, ChainDepthMutator, CodeGenMutator, CombineMutator, GcInjectionMutator,
    InputMutator, InterleaveMutator, LoadstringWrapMutator, MetamethodSwapMutator, Mutator,
    OperationMutator, SpliceMutator, TableSizeMutator,
};

const MIN_MUTATIONS_PER_ROUND: u32 = 1;
const MAX_MUTATIONS_PER_ROUND: u32 = 8;

struct WeightedMutator {
    mutator: Box<dyn Mutator>,
    weight: f64,
}

pub struct MutationEngine {
    mutators: Vec<WeightedMutator>,
}

impl MutationEngine {
    pub fn new() -> Self {
        Self::from_mutators(vec![
            (Box::new(InputMutator) as Box<dyn Mutator>, 20.0),
            (Box::new(OperationMutator), 25.0),
            (Box::new(SpliceMutator), 10.0),
            (Box::new(CombineMutator), 5.0),
            (Box::new(CodeGenMutator), 10.0),
            (Box::new(GcInjectionMutator), 20.0),
            (Box::new(ChainDepthMutator), 8.0),
            (Box::new(TableSizeMutator), 8.0),
            (Box::new(InterleaveMutator), 5.0),
            (Box::new(LoadstringWrapMutator), 5.0),
            (Box::new(CallbackGcMutator), 10.0),
            (Box::new(MetamethodSwapMutator), 8.0),
        ])
    }

    pub fn from_mutators(weighted: Vec<(Box<dyn Mutator>, f64)>) -> Self {
        let mutators = weighted
            .into_iter()
            .map(|(mutator, weight)| WeightedMutator { mutator, weight })
            .collect();
        Self { mutators }
    }

    pub fn mutate(&self, program: &mut Program, rng: &mut impl Rng) -> Vec<&'static str> {
        let num_mutations = rng.random_range(MIN_MUTATIONS_PER_ROUND..=MAX_MUTATIONS_PER_ROUND);
        let mut history = Vec::new();
        // Single upfront clone, then clone_from on success/failure. This reuses
        // Vec allocations across mutations instead of cloning per attempt
        // (was ~13% of CPU time from memmove in per-mutation clones).
        let mut snapshot = program.clone();

        for _ in 0..num_mutations {
            let Some(wm) = self.pick_mutator(rng) else {
                break;
            };

            if wm.mutator.mutate(program, rng) {
                if program.validate().is_ok() {
                    trace!(mutator = wm.mutator.name(), "mutation applied");
                    history.push(wm.mutator.name());
                    snapshot.clone_from(program);
                } else {
                    trace!(
                        mutator = wm.mutator.name(),
                        "mutation reverted (invalid IR)"
                    );
                    program.clone_from(&snapshot);
                }
            }
        }

        history
    }

    fn pick_mutator(&self, rng: &mut impl Rng) -> Option<&WeightedMutator> {
        if self.mutators.is_empty() {
            return None;
        }
        let total: f64 = self.mutators.iter().map(|wm| wm.weight).sum();
        let mut target = rng.random_range(0.0..total);
        for wm in &self.mutators {
            target -= wm.weight;
            if target <= 0.0 {
                return Some(wm);
            }
        }
        Some(self.mutators.last().unwrap())
    }
}

impl Default for MutationEngine {
    fn default() -> Self {
        Self::new()
    }
}
