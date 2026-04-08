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

pub fn find_balanced_splice_ranges(instructions: &[Instruction]) -> Vec<(usize, usize)> {
    let len = instructions.len();
    if len == 0 {
        return vec![];
    }

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

    #[allow(clippy::needless_range_loop)]
    for start in 0..len {
        for end in (start + 1)..=len {
            if block_depth[end] == block_depth[start]
                && is_self_contained(&instructions[start..end])
            {
                ranges.push((start, end));
            }
        }
    }

    ranges
}

fn is_self_contained(slice: &[Instruction]) -> bool {
    let mut defined = std::collections::HashSet::new();
    for instr in slice {
        for v in &instr.inputs {
            if !defined.contains(v) {
                return false;
            }
        }
        for v in &instr.outputs {
            defined.insert(*v);
        }
    }
    true
}
