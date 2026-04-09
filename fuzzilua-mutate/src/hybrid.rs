use fuzzilua_gen::{Generator, ProgramBuilder, ProgramTemplate, all_generators, all_templates};
use fuzzilua_ir::Program;
use rand::Rng;

use crate::MutationEngine;

const DEFAULT_TEMPLATE_BUDGET: usize = 200;
const DEFAULT_TEMPLATE_DEPTH: usize = 7;

pub struct HybridEngine {
    templates: Vec<Box<dyn ProgramTemplate>>,
    generators: Vec<Generator>,
    mutation_engine: MutationEngine,
}

impl HybridEngine {
    pub fn new() -> Self {
        Self {
            templates: all_templates(),
            generators: all_generators(),
            mutation_engine: MutationEngine::new(),
        }
    }

    pub fn generate(&self, rng: &mut impl Rng) -> Program {
        let template_idx = rng.random_range(0..self.templates.len());
        let template = &self.templates[template_idx];

        let mut builder = ProgramBuilder::new(DEFAULT_TEMPLATE_BUDGET, DEFAULT_TEMPLATE_DEPTH);
        template.generate(&mut builder, &self.generators, rng);
        let mut program = builder.finish();

        self.mutation_engine.mutate(&mut program, rng);

        program
    }
}

impl Default for HybridEngine {
    fn default() -> Self {
        Self::new()
    }
}
