use fuzzilua_gen::{ProgramBuilder, all_generators, generate_program};
use fuzzilua_ir::{BinOp, CmpOp, GcMode, LuaType, Op, Program, UnOp, Variable};
use rand::Rng;
use rand::RngCore;

use crate::util::{
    find_balanced_splice_ranges, find_compatible_variables, random_insertion_point, remap_variables,
};

const METAMETHODS: &[&str] = &[
    "__index",
    "__newindex",
    "__eq",
    "__concat",
    "__len",
    "__add",
    "__sub",
    "__mul",
    "__div",
    "__mod",
    "__pow",
    "__unm",
    "__lt",
    "__le",
    "__call",
    "__tostring",
    "__gc",
];

const CODEGEN_BUDGET: usize = 10;
const CODEGEN_MAX_DEPTH: usize = 2;
const TYPE_AWARE_PROBABILITY: f64 = 0.75;

pub trait Mutator {
    fn name(&self) -> &'static str;
    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool;
}

pub struct InputMutator;

impl Mutator for InputMutator {
    fn name(&self) -> &'static str {
        "InputMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        if program.instructions.is_empty() {
            return false;
        }

        let indices_with_inputs: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| !instr.inputs.is_empty())
            .map(|(i, _)| i)
            .collect();

        if indices_with_inputs.is_empty() {
            return false;
        }

        let target_idx = indices_with_inputs[rng.random_range(0..indices_with_inputs.len())];
        let input_slot = rng.random_range(0..program.instructions[target_idx].inputs.len());
        let old_var = program.instructions[target_idx].inputs[input_slot];

        let type_aware = rng.random_bool(TYPE_AWARE_PROBABILITY);
        let ty = if type_aware {
            Some(infer_var_type(program, old_var))
        } else {
            None
        };

        let candidates: Vec<Variable> = find_compatible_variables(program, target_idx, ty)
            .into_iter()
            .filter(|sv| sv.var != old_var)
            .map(|sv| sv.var)
            .collect();

        if candidates.is_empty() {
            return false;
        }

        let new_var = candidates[rng.random_range(0..candidates.len())];
        program.instructions[target_idx].inputs[input_slot] = new_var;
        true
    }
}

fn infer_var_type(program: &Program, var: Variable) -> LuaType {
    for instr in &program.instructions {
        if instr.outputs.contains(&var) {
            return Program::infer_output_type(&instr.op);
        }
    }
    LuaType::Anything
}

pub struct OperationMutator;

impl Mutator for OperationMutator {
    fn name(&self) -> &'static str {
        "OperationMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        if program.instructions.is_empty() {
            return false;
        }

        for _ in 0..10 {
            let idx = rng.random_range(0..program.instructions.len());
            if let Some(new_op) = mutate_op(&program.instructions[idx].op, rng) {
                program.instructions[idx].op = new_op;
                return true;
            }
        }
        false
    }
}

fn mutate_op(op: &Op, rng: &mut dyn RngCore) -> Option<Op> {
    match op {
        Op::BinaryOp(v) => {
            let variants = [
                BinOp::Add,
                BinOp::Sub,
                BinOp::Mul,
                BinOp::Div,
                BinOp::Mod,
                BinOp::Pow,
                BinOp::Concat,
            ];
            Some(Op::BinaryOp(pick_different(rng, &variants, v)))
        }
        Op::UnaryOp(v) => {
            let variants = [UnOp::Neg, UnOp::Not, UnOp::Len];
            Some(Op::UnaryOp(pick_different(rng, &variants, v)))
        }
        Op::Compare(v) => {
            let variants = [
                CmpOp::Eq,
                CmpOp::Ne,
                CmpOp::Lt,
                CmpOp::Le,
                CmpOp::Gt,
                CmpOp::Ge,
            ];
            Some(Op::Compare(pick_different(rng, &variants, v)))
        }
        Op::LoadInt(n) => {
            let delta: i64 = rng.random_range(-10..=10);
            Some(Op::LoadInt(n.wrapping_add(delta)))
        }
        Op::LoadFloat(n) => {
            let delta: f64 = rng.random_range(-10.0..=10.0);
            Some(Op::LoadFloat(n + delta))
        }
        Op::LoadBool(b) => Some(Op::LoadBool(!b)),
        Op::LoadString(s) => {
            let mut bytes: Vec<u8> = s.bytes().collect();
            if bytes.is_empty() {
                bytes.push(rng.random_range(b'a'..=b'z'));
            } else {
                let pos = rng.random_range(0..bytes.len());
                bytes[pos] ^= rng.random_range(1..=0x7Fu8);
            }
            Some(Op::LoadString(String::from_utf8_lossy(&bytes).into_owned()))
        }
        Op::TableSetField(name) => Some(Op::TableSetField(mutate_field_or_metamethod(name, rng))),
        Op::TableGetField(name) => Some(Op::TableGetField(mutate_field_or_metamethod(name, rng))),
        Op::CollectGarbage(mode) => {
            let variants = [GcMode::Collect, GcMode::Stop, GcMode::Restart, GcMode::Step];
            Some(Op::CollectGarbage(pick_different(rng, &variants, mode)))
        }
        _ => None,
    }
}

fn pick_different<T: Copy + PartialEq>(rng: &mut dyn RngCore, variants: &[T], current: &T) -> T {
    let others: Vec<T> = variants.iter().copied().filter(|v| v != current).collect();
    if others.is_empty() {
        return *current;
    }
    others[rng.random_range(0..others.len())]
}

fn mutate_field_or_metamethod(name: &str, rng: &mut dyn RngCore) -> String {
    if name.starts_with("__") {
        let others: Vec<&&str> = METAMETHODS.iter().filter(|m| **m != name).collect();
        if others.is_empty() {
            return name.to_string();
        }
        others[rng.random_range(0..others.len())].to_string()
    } else {
        let fields = [
            "x", "y", "z", "key", "value", "name", "data", "next", "prev", "count",
        ];
        let others: Vec<&&str> = fields.iter().filter(|f| **f != name).collect();
        if others.is_empty() {
            return name.to_string();
        }
        others[rng.random_range(0..others.len())].to_string()
    }
}

pub struct SpliceMutator;

impl Mutator for SpliceMutator {
    fn name(&self) -> &'static str {
        "SpliceMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let generators = all_generators();
        let donor = generate_program(rng, 20, 3, &generators);

        if donor.instructions.is_empty() {
            return false;
        }

        let splice_ranges = find_balanced_splice_ranges(&donor.instructions);
        if splice_ranges.is_empty() {
            return false;
        }

        let (start, end) = splice_ranges[rng.random_range(0..splice_ranges.len())];
        let splice_slice = &donor.instructions[start..end];

        let insert_pos = random_insertion_point(program, rng);
        let remapped = remap_variables(splice_slice, program.next_var);
        program.next_var += donor.next_var;

        let tail = program.instructions.split_off(insert_pos);
        program.instructions.extend(remapped);
        program.instructions.extend(tail);

        true
    }
}

pub struct CombineMutator;

impl Mutator for CombineMutator {
    fn name(&self) -> &'static str {
        "CombineMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let generators = all_generators();
        let donor = generate_program(rng, 20, 3, &generators);

        if donor.instructions.is_empty() {
            return false;
        }

        let remapped = remap_variables(&donor.instructions, program.next_var);
        program.next_var += donor.next_var;
        program.instructions.extend(remapped);

        true
    }
}

pub struct CodeGenMutator;

impl Mutator for CodeGenMutator {
    fn name(&self) -> &'static str {
        "CodeGenMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let insert_pos = random_insertion_point(program, rng);

        let scope_vars: Vec<(Variable, LuaType)> = program
            .variables_in_scope_at(insert_pos)
            .into_iter()
            .map(|sv| (sv.var, sv.lua_type))
            .collect();

        let generators = all_generators();
        let mut builder = ProgramBuilder::with_scope(
            CODEGEN_BUDGET,
            CODEGEN_MAX_DEPTH,
            program.next_var,
            scope_vars,
        );

        let mut generated = false;
        for _ in 0..5 {
            let generator = &generators[rng.random_range(0..generators.len())];
            if (generator.generate)(&mut builder, rng).is_some() {
                generated = true;
                break;
            }
        }

        if !generated {
            return false;
        }

        let fragment = builder.finish();
        if fragment.instructions.is_empty() {
            return false;
        }

        let old_next = program.next_var;
        program.next_var = fragment.next_var.max(old_next);

        let tail = program.instructions.split_off(insert_pos);
        program.instructions.extend(fragment.instructions);
        program.instructions.extend(tail);

        true
    }
}

pub struct GcInjectionMutator;

impl Mutator for GcInjectionMutator {
    fn name(&self) -> &'static str {
        "GcInjectionMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let insert_pos = random_insertion_point(program, rng);
        let mode = if rng.random_bool(0.5) {
            GcMode::Collect
        } else {
            GcMode::Step
        };

        let instr = fuzzilua_ir::Instruction {
            op: Op::CollectGarbage(mode),
            inputs: vec![],
            outputs: vec![],
        };

        program.instructions.insert(insert_pos, instr);
        true
    }
}
