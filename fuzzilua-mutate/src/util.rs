use fuzzilua_ir::{Instruction, LuaType, Program, ScopedVariable, Variable};
use rand::{Rng, RngCore};

pub fn remap_variables(instructions: &[Instruction], offset: u32) -> Vec<Instruction> {
    instructions
        .iter()
        .map(|instr| {
            let inputs = instr
                .inputs
                .iter()
                .map(|v| Variable(v.0 + offset))
                .collect();
            let outputs = instr
                .outputs
                .iter()
                .map(|v| Variable(v.0 + offset))
                .collect();
            Instruction {
                op: instr.op.clone(),
                inputs,
                outputs,
            }
        })
        .collect()
}

pub fn find_compatible_variables(
    program: &Program,
    index: usize,
    ty: Option<LuaType>,
) -> Vec<ScopedVariable> {
    let in_scope = program.variables_in_scope_at(index);
    match ty {
        Some(target) => in_scope
            .into_iter()
            .filter(|sv| {
                sv.lua_type == target
                    || sv.lua_type == LuaType::Anything
                    || target == LuaType::Anything
            })
            .collect(),
        None => in_scope,
    }
}

pub fn random_insertion_point(program: &Program, rng: &mut dyn RngCore) -> usize {
    rng.random_range(0..=program.instructions.len())
}

const MAX_SPLICE_RANGES: usize = 500;

/// Finds self-contained, block-balanced sub-ranges for splicing.
/// Ensures the block depth never drops below the starting depth within a range,
/// preventing extraction of partial blocks (e.g. an EndIf without its BeginIf).
pub fn find_balanced_splice_ranges(instructions: &[Instruction]) -> Vec<(usize, usize)> {
    let len = instructions.len();
    if len == 0 {
        return vec![];
    }

    let max_var = instructions
        .iter()
        .flat_map(|i| i.outputs.iter().chain(i.inputs.iter()))
        .map(|v| v.0 as usize)
        .max()
        .unwrap_or(0);

    let mut ranges = Vec::new();
    let mut block_depth = vec![0i32; len + 1];

    let mut depth = 0i32;
    for (i, instr) in instructions.iter().enumerate() {
        if instr.op.opens_block().is_some() {
            depth += 1;
        }
        if instr.op.closes_block().is_some() {
            depth -= 1;
        }
        block_depth[i + 1] = depth;
    }

    // Generation-counter approach: bumping `gen` each outer iteration avoids
    // clearing the whole array. A variable is "defined" when its entry == gen.
    let mut defined_gen = vec![0u32; max_var + 1];
    let mut generation = 0u32;

    'outer: for start in 0..len {
        generation = generation.wrapping_add(1);
        if generation == 0 {
            defined_gen.fill(0);
            generation = 1;
        }

        let start_depth = block_depth[start];
        let mut min_depth = start_depth;

        for (end_idx, instr) in instructions[start..].iter().enumerate() {
            let end_idx = start + end_idx;

            for v in &instr.inputs {
                if defined_gen[v.0 as usize] != generation {
                    continue 'outer;
                }
            }

            for v in &instr.outputs {
                defined_gen[v.0 as usize] = generation;
            }

            let end = end_idx + 1;
            min_depth = min_depth.min(block_depth[end]);

            if block_depth[end] == start_depth && min_depth >= start_depth {
                ranges.push((start, end));
                if ranges.len() >= MAX_SPLICE_RANGES {
                    return ranges;
                }
            }
        }
    }

    ranges
}
