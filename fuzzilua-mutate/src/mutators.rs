use fuzzilua_gen::{ProgramBuilder, all_generators, generate_program};
use fuzzilua_ir::{
    BinOp, CmpOp, GcMode, Instruction, LuaType, METAMETHODS, Op, Program, UnOp, VarBitset, Variable,
};
use rand::Rng;
use rand::RngCore;
use std::sync::Arc;

use crate::util::{
    find_balanced_splice_ranges, find_compatible_variables, random_insertion_point, remap_variables,
};

const CODEGEN_BUDGET: usize = 10;
const CODEGEN_MAX_DEPTH: usize = 2;
const TYPE_AWARE_PROBABILITY: f64 = 0.75;

pub trait Mutator: Send + Sync {
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
            Some(Op::LoadString(
                String::from_utf8_lossy(&bytes).into_owned().into(),
            ))
        }
        Op::TableSetField(name) => Some(Op::TableSetField(
            mutate_field_or_metamethod(name, rng).into(),
        )),
        Op::TableGetField(name) => Some(Op::TableGetField(
            mutate_field_or_metamethod(name, rng).into(),
        )),
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
        let others: Vec<&str> = METAMETHODS
            .iter()
            .map(|m| m.name)
            .filter(|n| *n != name)
            .collect();
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

const REHASH_BOUNDARIES: &[i64] = &[0, 1, 8, 16, 32];

pub struct ChainDepthMutator;

impl Mutator for ChainDepthMutator {
    fn name(&self) -> &'static str {
        "ChainDepthMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let chain_starts: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| matches!(instr.op, Op::SetMetatable))
            .map(|(i, _)| i)
            .collect();

        if chain_starts.is_empty() {
            return false;
        }

        let idx = chain_starts[rng.random_range(0..chain_starts.len())];
        let result_var = program.instructions[idx].outputs[0];
        let table_var = program.instructions[idx].inputs[0];

        if rng.random_bool(0.7) {
            let new_table = Variable(program.next_var);
            let new_mt = Variable(program.next_var + 1);
            let new_result = Variable(program.next_var + 2);
            program.next_var += 3;

            let new_instrs = vec![
                Instruction {
                    op: Op::CreateTable,
                    inputs: vec![],
                    outputs: vec![new_table],
                },
                Instruction {
                    op: Op::CreateTable,
                    inputs: vec![],
                    outputs: vec![new_mt],
                },
                Instruction {
                    op: Op::TableSetField("__index".into()),
                    inputs: vec![new_mt, result_var],
                    outputs: vec![],
                },
                Instruction {
                    op: Op::SetMetatable,
                    inputs: vec![new_table, new_mt],
                    outputs: vec![new_result],
                },
            ];

            let insert_at = idx + 1;
            let tail = program.instructions.split_off(insert_at);
            program.instructions.extend(new_instrs);
            program.instructions.extend(tail);
        } else {
            if idx + 1 < program.instructions.len() {
                let next = &program.instructions[idx + 1];
                if matches!(next.op, Op::SetMetatable) && next.inputs[0] != table_var {
                    return false;
                }
            }
            for instr in program.instructions.iter_mut().skip(idx + 1) {
                for v in instr.inputs.iter_mut() {
                    if *v == result_var {
                        *v = table_var;
                    }
                }
            }
            program.instructions[idx] = Instruction::nop();
        }

        true
    }
}

pub struct TableSizeMutator;

impl Mutator for TableSizeMutator {
    fn name(&self) -> &'static str {
        "TableSizeMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let table_creates: Vec<(usize, Variable)> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| matches!(instr.op, Op::CreateTable))
            .map(|(i, instr)| (i, instr.outputs[0]))
            .collect();

        if table_creates.is_empty() {
            return false;
        }

        let (create_idx, table_var) = table_creates[rng.random_range(0..table_creates.len())];

        let target_size = REHASH_BOUNDARIES[rng.random_range(0..REHASH_BOUNDARIES.len())];

        let current_count: usize = program
            .instructions
            .iter()
            .skip(create_idx + 1)
            .filter(|instr| {
                matches!(
                    instr.op,
                    Op::TableSetNumericField(_) | Op::TableSetField(_) | Op::TableSetIndex
                ) && instr.inputs.first() == Some(&table_var)
            })
            .count();

        let current = current_count as i64;

        if current == target_size {
            return false;
        }

        if target_size > current {
            let to_add = (target_size - current) as usize;
            let insert_at = (create_idx + 1).min(program.instructions.len());

            let mut new_instrs = Vec::with_capacity(to_add * 2);
            for i in 0..to_add {
                let val_var = Variable(program.next_var);
                program.next_var += 1;
                new_instrs.push(Instruction {
                    op: Op::LoadInt(rng.random_range(0..100)),
                    inputs: vec![],
                    outputs: vec![val_var],
                });
                new_instrs.push(Instruction {
                    op: Op::TableSetNumericField(current + i as i64 + 1),
                    inputs: vec![table_var, val_var],
                    outputs: vec![],
                });
            }

            let tail = program.instructions.split_off(insert_at);
            program.instructions.extend(new_instrs);
            program.instructions.extend(tail);
        } else {
            let to_remove = (current - target_size) as usize;
            let mut removed = 0;
            for i in (create_idx + 1..program.instructions.len()).rev() {
                if removed >= to_remove {
                    break;
                }
                let is_set = matches!(
                    program.instructions[i].op,
                    Op::TableSetNumericField(_) | Op::TableSetField(_) | Op::TableSetIndex
                ) && program.instructions[i].inputs.first() == Some(&table_var);
                if is_set {
                    program.instructions[i] = Instruction::nop();
                    removed += 1;
                }
            }
        }

        true
    }
}

pub struct InterleaveMutator;

impl Mutator for InterleaveMutator {
    fn name(&self) -> &'static str {
        "InterleaveMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let generators = all_generators();
        let donor = generate_program(rng, 20, 3, &generators);
        if donor.instructions.is_empty() {
            return false;
        }

        let donor_ranges = find_balanced_splice_ranges(&donor.instructions);
        if donor_ranges.is_empty() {
            return false;
        }
        let (ds, de) = donor_ranges[rng.random_range(0..donor_ranges.len())];
        let donor_slice = remap_variables(&donor.instructions[ds..de], program.next_var);
        program.next_var += donor.next_var;

        let host_ranges = find_balanced_splice_ranges(&program.instructions);
        let usable: Vec<_> = host_ranges
            .into_iter()
            .filter(|(s, e)| e - s >= 2)
            .collect();

        let interleaved = if let Some(&(hs, he)) = usable.first() {
            let host_slice: Vec<_> = program.instructions[hs..he].to_vec();
            interleave_slices(&host_slice, &donor_slice, rng)
        } else {
            donor_slice
        };

        let insert_pos = random_insertion_point(program, rng);
        let tail = program.instructions.split_off(insert_pos);
        program.instructions.extend(interleaved);
        program.instructions.extend(tail);

        true
    }
}

fn interleave_slices(
    a: &[Instruction],
    b: &[Instruction],
    rng: &mut dyn RngCore,
) -> Vec<Instruction> {
    let chunks_a = split_into_block_aware_chunks(a);
    let chunks_b = split_into_block_aware_chunks(b);

    let mut result = Vec::new();
    let mut ia = 0;
    let mut ib = 0;

    while ia < chunks_a.len() && ib < chunks_b.len() {
        if rng.random_bool(0.5) {
            result.extend_from_slice(&chunks_a[ia]);
            ia += 1;
        } else {
            result.extend_from_slice(&chunks_b[ib]);
            ib += 1;
        }
    }
    for chunk in &chunks_a[ia..] {
        result.extend_from_slice(chunk);
    }
    for chunk in &chunks_b[ib..] {
        result.extend_from_slice(chunk);
    }

    result
}

fn split_into_block_aware_chunks(instrs: &[Instruction]) -> Vec<Vec<Instruction>> {
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < instrs.len() {
        if instrs[i].op.opens_block().is_some() {
            let start = i;
            let mut depth = 1i32;
            i += 1;
            while i < instrs.len() && depth > 0 {
                if instrs[i].op.opens_block().is_some() {
                    depth += 1;
                }
                if instrs[i].op.closes_block().is_some() {
                    depth -= 1;
                }
                i += 1;
            }
            chunks.push(instrs[start..i].to_vec());
        } else {
            chunks.push(vec![instrs[i].clone()]);
            i += 1;
        }
    }
    chunks
}

pub struct LoadstringWrapMutator;

impl Mutator for LoadstringWrapMutator {
    fn name(&self) -> &'static str {
        "LoadstringWrapMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let ranges = find_balanced_splice_ranges(&program.instructions);
        let usable: Vec<_> = ranges
            .into_iter()
            .filter(|(s, e)| {
                let len = e - s;
                if !(1..=20).contains(&len) {
                    return false;
                }
                let slice = &program.instructions[*s..*e];
                if slice
                    .iter()
                    .any(|i| matches!(i.op, Op::Return | Op::Break | Op::CoroutineYield))
                {
                    return false;
                }
                let mut defined_in_slice = VarBitset::new();
                for instr in slice {
                    for v in &instr.outputs {
                        defined_in_slice.insert(*v);
                    }
                }
                let used_after = program.instructions[*e..]
                    .iter()
                    .any(|i| i.inputs.iter().any(|v| defined_in_slice.contains(v)));
                !used_after
            })
            .collect();

        if usable.is_empty() {
            return false;
        }

        let (start, end) = usable[rng.random_range(0..usable.len())];
        let slice = &program.instructions[start..end];

        let sub_program = Program {
            instructions: slice.to_vec(),
            next_var: program.next_var,
        };
        let lua_inner = fuzzilua_ir::lift(&sub_program);
        if lua_inner.trim().is_empty() {
            return false;
        }

        let escaped = lua_inner
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\0', "\\0");

        let wrapper = format!("local __f = loadstring(\"{escaped}\") if __f then __f() end");

        let code_var = Variable(program.next_var);
        let loader_var = Variable(program.next_var + 1);
        program.next_var += 2;

        let new_instrs = vec![
            Instruction {
                op: Op::LoadString(wrapper.into()),
                inputs: vec![],
                outputs: vec![code_var],
            },
            Instruction {
                op: Op::Loadstring,
                inputs: vec![code_var],
                outputs: vec![loader_var],
            },
            Instruction {
                op: Op::CallFunction {
                    arg_count: 0,
                    ret_count: 0,
                },
                inputs: vec![loader_var],
                outputs: vec![],
            },
        ];

        let tail = program.instructions.split_off(end);
        program.instructions.truncate(start);
        program.instructions.extend(new_instrs);
        program.instructions.extend(tail);

        true
    }
}

pub struct MetamethodSwapMutator;

impl Mutator for MetamethodSwapMutator {
    fn name(&self) -> &'static str {
        "MetamethodSwapMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let mm_indices: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(
                |(_, instr)| matches!(&instr.op, Op::TableSetField(name) if name.starts_with("__")),
            )
            .map(|(i, _)| i)
            .collect();

        if mm_indices.is_empty() {
            return false;
        }

        let idx = mm_indices[rng.random_range(0..mm_indices.len())];
        if let Op::TableSetField(ref name) = program.instructions[idx].op {
            let new_name: Arc<str> = mutate_field_or_metamethod(name, rng).into();
            if *new_name == **name {
                return false;
            }
            program.instructions[idx].op = Op::TableSetField(new_name);
            true
        } else {
            false
        }
    }
}

pub struct CallbackGcMutator;

impl Mutator for CallbackGcMutator {
    fn name(&self) -> &'static str {
        "CallbackGcMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let mut depth = 0i32;
        let mut callback_bodies = Vec::new();

        for (i, instr) in program.instructions.iter().enumerate() {
            if instr.op.closes_block().is_some() {
                depth -= 1;
            }
            if matches!(instr.op, Op::BeginFunction { .. })
                && let Some(fn_var) = instr.outputs.first()
            {
                let is_metamethod_value = program.instructions.iter().any(|later| {
                    matches!(&later.op, Op::TableSetField(name) if name.starts_with("__"))
                        && later.inputs.len() >= 2
                        && later.inputs[1] == *fn_var
                });
                let is_callback_arg = program.instructions.iter().any(|later| {
                    matches!(&later.op, Op::CallFunction { .. })
                        && later.inputs.len() >= 2
                        && later.inputs[1..].contains(fn_var)
                });
                let is_nested = depth > 0;
                if is_metamethod_value || is_callback_arg || is_nested {
                    callback_bodies.push(i);
                }
            }
            if instr.op.opens_block().is_some() {
                depth += 1;
            }
        }

        if callback_bodies.is_empty() {
            return false;
        }

        let target_idx = callback_bodies[rng.random_range(0..callback_bodies.len())];

        let already_has_gc = target_idx + 1 < program.instructions.len()
            && matches!(
                program.instructions[target_idx + 1].op,
                Op::CollectGarbage(_)
            );
        if already_has_gc {
            return false;
        }

        let gc_instr = Instruction {
            op: Op::CollectGarbage(GcMode::Collect),
            inputs: vec![],
            outputs: vec![],
        };

        program.instructions.insert(target_idx + 1, gc_instr);
        true
    }
}
