use fuzzilua_gen::{ProgramBuilder, all_generators, generate_program};
use fuzzilua_ir::{
    BinOp, CmpOp, GcMode, Instruction, LuaType, METAMETHODS, Op, Program, UnOp, VarBitset, Variable,
};
use rand::{Rng, RngCore};
use std::sync::Arc;

use crate::util::{
    find_balanced_splice_ranges, find_block_end, find_compatible_variables, find_fn_blocks,
    is_metamethod_fn, pick_random, random_insertion_point, remap_variables,
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
            let end = find_block_end(instrs, i).unwrap_or(instrs.len());
            chunks.push(instrs[i..end].to_vec());
            i = end;
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

pub struct InstructionDeleteMutator;

impl Mutator for InstructionDeleteMutator {
    fn name(&self) -> &'static str {
        "InstructionDeleteMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        if program.instructions.len() < 3 {
            return false;
        }

        for _ in 0..10 {
            let idx = rng.random_range(0..program.instructions.len());
            let instr = &program.instructions[idx];

            if instr.op.opens_block().is_some() || instr.op.closes_block().is_some() {
                continue;
            }

            let any_output_used = instr.outputs.iter().any(|out| {
                program.instructions[idx + 1..]
                    .iter()
                    .any(|later| later.inputs.contains(out))
            });
            if any_output_used {
                continue;
            }

            program.instructions.remove(idx);
            return true;
        }
        false
    }
}

pub struct TypeConfusionMutator;

impl Mutator for TypeConfusionMutator {
    fn name(&self) -> &'static str {
        "TypeConfusionMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        if program.instructions.len() < 2 {
            return false;
        }

        let strategy = rng.random_range(0..3u8);
        match strategy {
            0 => self.reassign_wrong_type(program, rng),
            1 => self.metamethod_wrong_return(program, rng),
            _ => self.swap_typed_inputs(program, rng),
        }
    }
}

impl TypeConfusionMutator {
    fn reassign_wrong_type(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let typed_uses: Vec<(usize, usize)> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| !instr.inputs.is_empty() && instr.op.is_effectful())
            .flat_map(|(i, instr)| (0..instr.inputs.len()).map(move |slot| (i, slot)))
            .collect();

        if typed_uses.is_empty() {
            return false;
        }

        let &(use_idx, slot) = pick_random(&typed_uses, rng);
        let target_var = program.instructions[use_idx].inputs[slot];
        let current_type = infer_var_type(program, target_var);

        let confusion_op = pick_confusing_value(current_type, rng);
        let new_var = program.new_var();

        let load_instr = Instruction {
            op: confusion_op,
            inputs: vec![],
            outputs: vec![new_var],
        };
        let reassign_instr = Instruction {
            op: Op::Reassign,
            inputs: vec![new_var],
            outputs: vec![target_var],
        };

        let tail = program.instructions.split_off(use_idx);
        program.instructions.push(load_instr);
        program.instructions.push(reassign_instr);
        program.instructions.extend(tail);
        true
    }

    fn metamethod_wrong_return(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let mm_fns: Vec<(usize, usize)> = find_fn_blocks(program)
            .into_iter()
            .filter(|&(_, _, fv)| is_metamethod_fn(program, fv))
            .map(|(start, end, _)| (start, end))
            .collect();

        if mm_fns.is_empty() {
            return false;
        }

        let &(fn_start, fn_end) = pick_random(&mm_fns, rng);
        let closer_idx = fn_end - 1;

        let return_indices: Vec<usize> = (fn_start..closer_idx)
            .filter(|&j| matches!(program.instructions[j].op, Op::Return))
            .collect();

        if return_indices.is_empty() {
            let wrong_val = program.new_var();

            let wrong_type = pick_confusing_value(LuaType::Table, rng);
            let load = Instruction {
                op: wrong_type,
                inputs: vec![],
                outputs: vec![wrong_val],
            };
            let ret = Instruction {
                op: Op::Return,
                inputs: vec![wrong_val],
                outputs: vec![],
            };

            let tail = program.instructions.split_off(closer_idx);
            program.instructions.push(load);
            program.instructions.push(ret);
            program.instructions.extend(tail);
        } else {
            let &ret_idx = pick_random(&return_indices, rng);
            if !program.instructions[ret_idx].inputs.is_empty() {
                let ret_var = program.instructions[ret_idx].inputs[0];
                let current_type = infer_var_type(program, ret_var);
                let wrong_op = pick_confusing_value(current_type, rng);
                let new_var = program.new_var();
                let load = Instruction {
                    op: wrong_op,
                    inputs: vec![],
                    outputs: vec![new_var],
                };
                program.instructions.insert(ret_idx, load);
                program.instructions[ret_idx + 1].inputs[0] = new_var;
            }
        }
        true
    }

    fn swap_typed_inputs(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let multi_input: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| instr.inputs.len() >= 2)
            .map(|(i, _)| i)
            .collect();

        if multi_input.is_empty() {
            return false;
        }

        let idx = *pick_random(&multi_input, rng);
        let len = program.instructions[idx].inputs.len();
        let a = rng.random_range(0..len);
        let mut b = rng.random_range(0..len - 1);
        if b >= a {
            b += 1;
        }
        let type_a = infer_var_type(program, program.instructions[idx].inputs[a]);
        let type_b = infer_var_type(program, program.instructions[idx].inputs[b]);
        if type_a == type_b {
            return false;
        }
        program.instructions[idx].inputs.swap(a, b);
        true
    }
}

fn pick_confusing_value(current: LuaType, rng: &mut dyn RngCore) -> Op {
    let confused: &[Op] = match current {
        LuaType::Table | LuaType::Function | LuaType::Coroutine => &[
            Op::LoadInt(0),
            Op::LoadString("".into()),
            Op::LoadBool(false),
            Op::LoadFloat(f64::NAN),
            Op::LoadNil,
        ],
        LuaType::Integer | LuaType::Number => &[
            Op::CreateTable,
            Op::LoadString("not_a_number".into()),
            Op::LoadBool(true),
            Op::LoadNil,
        ],
        LuaType::String => &[
            Op::CreateTable,
            Op::LoadInt(0xDEAD),
            Op::LoadBool(false),
            Op::LoadNil,
            Op::LoadFloat(f64::INFINITY),
        ],
        LuaType::Boolean => &[
            Op::LoadInt(0),
            Op::LoadString("".into()),
            Op::LoadNil,
            Op::CreateTable,
        ],
        _ => &[
            Op::CreateTable,
            Op::LoadInt(-1),
            Op::LoadString("confused".into()),
            Op::LoadFloat(f64::NAN),
            Op::LoadBool(false),
            Op::LoadNil,
        ],
    };
    pick_random(confused, rng).clone()
}

pub struct PcallWrapMutator;

impl Mutator for PcallWrapMutator {
    fn name(&self) -> &'static str {
        "PcallWrapMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        if rng.random_bool(0.6) {
            self.wrap_in_pcall(program, rng)
        } else {
            self.unwrap_pcall(program, rng)
        }
    }
}

impl PcallWrapMutator {
    fn wrap_in_pcall(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let ranges = find_balanced_splice_ranges(&program.instructions);
        let usable: Vec<_> = ranges
            .into_iter()
            .filter(|(s, e)| {
                let len = e - s;
                (1..=15).contains(&len)
                    && !program.instructions[*s..*e]
                        .iter()
                        .any(|i| matches!(i.op, Op::Return | Op::Break))
            })
            .collect();

        if usable.is_empty() {
            return false;
        }

        let &(start, end) = pick_random(&usable, rng);

        let status_var = program.new_var();

        let begin = Instruction {
            op: Op::BeginPcall,
            inputs: vec![],
            outputs: vec![status_var],
        };
        let end_instr = Instruction {
            op: Op::EndPcall,
            inputs: vec![],
            outputs: vec![],
        };

        program.instructions.insert(end, end_instr);
        program.instructions.insert(start, begin);
        true
    }

    fn unwrap_pcall(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let pcall_begins: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| matches!(instr.op, Op::BeginPcall))
            .map(|(i, _)| i)
            .collect();

        if pcall_begins.is_empty() {
            return false;
        }

        let begin_idx = *pick_random(&pcall_begins, rng);
        let Some(end_past) = find_block_end(&program.instructions, begin_idx) else {
            return false;
        };

        program.instructions.remove(end_past - 1);
        program.instructions.remove(begin_idx);
        true
    }
}

pub struct CoroutineYieldInjectionMutator;

impl Mutator for CoroutineYieldInjectionMutator {
    fn name(&self) -> &'static str {
        "CoroutineYieldInjectionMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        // Find function blocks that are used as metamethods or callbacks
        let fn_blocks = find_fn_blocks(program);
        if fn_blocks.is_empty() {
            return false;
        }

        let interesting: Vec<_> = fn_blocks
            .iter()
            .copied()
            .filter(|&(_, _, fv)| {
                is_metamethod_fn(program, fv)
                    || program.instructions.iter().any(|instr| {
                        matches!(&instr.op, Op::CallFunction { .. })
                            && instr.inputs.len() >= 2
                            && instr.inputs[1..].contains(&fv)
                    })
            })
            .collect();

        let targets = if !interesting.is_empty() {
            &interesting
        } else {
            &fn_blocks
        };

        let &(fn_start, fn_end, _) = pick_random(targets, rng);

        // Don't inject if there's already a yield in this function body
        let has_yield = (fn_start..fn_end)
            .any(|i| matches!(program.instructions[i].op, Op::CoroutineYield));
        if has_yield {
            return false;
        }

        // Insert yield + GC right after the function header
        let insert_at = fn_start + 1;
        if insert_at >= fn_end {
            return false;
        }

        let mut new_instrs = vec![
            Instruction {
                op: Op::CoroutineYield,
                inputs: vec![],
                outputs: vec![],
            },
            Instruction {
                op: Op::CollectGarbage(GcMode::Collect),
                inputs: vec![],
                outputs: vec![],
            },
        ];

        // Sometimes also add allocation pressure
        if rng.random_bool(0.5) {
            new_instrs.push(Instruction {
                op: Op::CreateTable,
                inputs: vec![],
                outputs: vec![Variable(program.next_var)],
            });
            program.next_var += 1;
        }

        let tail = program.instructions.split_off(insert_at);
        program.instructions.extend(new_instrs);
        program.instructions.extend(tail);
        true
    }
}

// ---------------------------------------------------------------------------
// LoadstringContentMutator — mutates Lua source inside loadstring() calls
// ---------------------------------------------------------------------------

pub struct LoadstringContentMutator;

impl Mutator for LoadstringContentMutator {
    fn name(&self) -> &'static str {
        "LoadstringContentMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        // Find variables that feed into Loadstring instructions
        let loadstring_inputs: Vec<Variable> = program
            .instructions
            .iter()
            .filter(|instr| matches!(instr.op, Op::Loadstring))
            .filter_map(|instr| instr.inputs.first().copied())
            .collect();

        if loadstring_inputs.is_empty() {
            return false;
        }

        // Find LoadString instructions that produce those variables (i.e., embedded Lua code)
        let candidates: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instr)| {
                matches!(&instr.op, Op::LoadString(s) if s.len() > 20)
                    && instr
                        .outputs
                        .first()
                        .is_some_and(|v| loadstring_inputs.contains(v))
            })
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return false;
        }

        let idx = candidates[rng.random_range(0..candidates.len())];
        let Op::LoadString(ref s) = program.instructions[idx].op else {
            return false;
        };

        let lua = s.to_string();
        let mutated = match rng.random_range(0..8u8) {
            0 | 1 => ls_insert_gc(&lua, rng),
            2 => ls_change_number(&lua, rng),
            3 => ls_insert_alloc(&lua, rng),
            4 => ls_swap_gc_mode(&lua, rng),
            5 => ls_remove_gc(&lua, rng),
            6 => ls_duplicate_gc(&lua, rng),
            _ => ls_insert_yield(&lua, rng),
        };

        if mutated == lua || mutated.is_empty() {
            return false;
        }

        program.instructions[idx].op = Op::LoadString(mutated.into());
        true
    }
}

/// Find positions in embedded Lua source where a standalone statement can be inserted.
fn ls_find_insertion_points(lua: &str) -> Vec<usize> {
    let mut points = Vec::new();
    let bytes = lua.as_bytes();

    // After "end " preceded by non-alphanumeric (keyword boundary)
    for (i, _) in lua.match_indices("end ") {
        if i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
            points.push(i + 4);
        }
    }
    // After "do " preceded by space
    for (i, _) in lua.match_indices("do ") {
        if i > 0 && bytes[i - 1] == b' ' {
            points.push(i + 3);
        }
    }
    // After "then "
    for (i, _) in lua.match_indices("then ") {
        if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() {
            points.push(i + 5);
        }
    }

    // Fallback: before a trailing "end"
    if points.is_empty() {
        if let Some(pos) = lua.rfind(" end") {
            points.push(pos + 1);
        }
    }

    points.sort();
    points.dedup();
    points
}

fn ls_insert_gc(lua: &str, rng: &mut dyn RngCore) -> String {
    let points = ls_find_insertion_points(lua);
    if points.is_empty() {
        return lua.to_string();
    }
    let pos = points[rng.random_range(0..points.len())];
    let gc = if rng.random_bool(0.5) {
        "collectgarbage('collect') "
    } else {
        "collectgarbage('step') "
    };
    format!("{}{}{}", &lua[..pos], gc, &lua[pos..])
}

fn ls_insert_alloc(lua: &str, rng: &mut dyn RngCore) -> String {
    let points = ls_find_insertion_points(lua);
    if points.is_empty() {
        return lua.to_string();
    }
    let pos = points[rng.random_range(0..points.len())];
    let snippet = match rng.random_range(0..3u8) {
        0 => {
            let n = rng.random_range(1..=20);
            format!("for __i=1,{n} do local __t={{}} end ")
        }
        1 => {
            let n = rng.random_range(10..=1000);
            format!("local __s=string.rep('x',{n}) ")
        }
        _ => "local __t={} ".to_string(),
    };
    format!("{}{}{}", &lua[..pos], snippet, &lua[pos..])
}

fn ls_change_number(lua: &str, rng: &mut dyn RngCore) -> String {
    let bytes = lua.as_bytes();
    let mut numbers: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Skip if part of an identifier
            if start > 0
                && (bytes[start - 1].is_ascii_alphabetic() || bytes[start - 1] == b'_')
            {
                continue;
            }
            numbers.push((start, i));
        } else {
            i += 1;
        }
    }

    if numbers.is_empty() {
        return lua.to_string();
    }

    let (start, end) = numbers[rng.random_range(0..numbers.len())];
    let old_num: i64 = lua[start..end].parse().unwrap_or(0);

    let new_num = match rng.random_range(0..6u8) {
        0 => old_num.saturating_add(1),
        1 => old_num.saturating_sub(1).max(0),
        2 => old_num.saturating_mul(2),
        3 => 0,
        4 => 1,
        _ => {
            let boundaries: &[i64] = &[0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 100, 1000];
            boundaries[rng.random_range(0..boundaries.len())]
        }
    };

    format!("{}{}{}", &lua[..start], new_num, &lua[end..])
}

fn ls_swap_gc_mode(lua: &str, rng: &mut dyn RngCore) -> String {
    let collects: Vec<usize> = lua
        .match_indices("collectgarbage('collect')")
        .map(|(i, _)| i)
        .collect();
    let steps: Vec<usize> = lua
        .match_indices("collectgarbage('step')")
        .map(|(i, _)| i)
        .collect();

    let all: Vec<(usize, bool)> = collects
        .iter()
        .map(|&i| (i, true))
        .chain(steps.iter().map(|&i| (i, false)))
        .collect();

    if all.is_empty() {
        return lua.to_string();
    }

    let &(pos, is_collect) = &all[rng.random_range(0..all.len())];

    if is_collect {
        let old = "collectgarbage('collect')";
        let new_call = "collectgarbage('step')";
        format!("{}{}{}", &lua[..pos], new_call, &lua[pos + old.len()..])
    } else {
        let old = "collectgarbage('step')";
        let new_call = "collectgarbage('collect')";
        format!("{}{}{}", &lua[..pos], new_call, &lua[pos + old.len()..])
    }
}

fn ls_remove_gc(lua: &str, rng: &mut dyn RngCore) -> String {
    let gc_calls: Vec<(usize, usize)> = lua
        .match_indices("collectgarbage(")
        .filter_map(|(start, _)| {
            lua[start..].find(')').map(|end| (start, start + end + 1))
        })
        .collect();

    if gc_calls.is_empty() {
        return lua.to_string();
    }

    let (start, end) = gc_calls[rng.random_range(0..gc_calls.len())];
    // Also consume trailing space
    let end = if end < lua.len() && lua.as_bytes()[end] == b' ' {
        end + 1
    } else {
        end
    };
    format!("{}{}", &lua[..start], &lua[end..])
}

fn ls_duplicate_gc(lua: &str, rng: &mut dyn RngCore) -> String {
    let gc_calls: Vec<(usize, usize)> = lua
        .match_indices("collectgarbage(")
        .filter_map(|(start, _)| {
            lua[start..].find(')').map(|end| (start, start + end + 1))
        })
        .collect();

    if gc_calls.is_empty() {
        return lua.to_string();
    }

    let (start, end) = gc_calls[rng.random_range(0..gc_calls.len())];
    let call = lua[start..end].to_string();
    format!("{} {}{}", &lua[..end], call, &lua[end..])
}

fn ls_insert_yield(lua: &str, rng: &mut dyn RngCore) -> String {
    let points = ls_find_insertion_points(lua);
    if points.is_empty() {
        return lua.to_string();
    }
    let pos = points[rng.random_range(0..points.len())];
    format!("{}coroutine.yield() {}", &lua[..pos], &lua[pos..])
}

// ---------------------------------------------------------------------------
// EnvironmentMutator
// ---------------------------------------------------------------------------

pub struct EnvironmentMutator;

impl Mutator for EnvironmentMutator {
    fn name(&self) -> &'static str {
        "EnvironmentMutator"
    }

    fn mutate(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let strategy = rng.random_range(0..3u8);
        match strategy {
            0 => self.setfenv_with_metamethods(program, rng),
            1 => self.swap_function_env(program, rng),
            _ => self.setfenv_raw(program, rng),
        }
    }
}

impl EnvironmentMutator {
    fn setfenv_with_metamethods(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let fn_blocks = find_fn_blocks(program);
        if fn_blocks.is_empty() {
            return false;
        }

        let &(_, fn_end, fn_var) = pick_random(&fn_blocks, rng);

        let env_table = program.new_var();
        let mt = program.new_var();
        let index_fn_var = program.new_var();
        let index_self = program.new_var();
        let index_key = program.new_var();
        let result = program.new_var();
        let mt_result = program.new_var();

        let index_body_op: Op = if rng.random_bool(0.5) {
            Op::LoadInt(rng.random_range(0..100))
        } else {
            Op::LoadString("polluted".into())
        };

        let new_instrs = vec![
            Instruction {
                op: Op::CreateTable,
                inputs: vec![],
                outputs: vec![env_table],
            },
            Instruction {
                op: Op::CreateTable,
                inputs: vec![],
                outputs: vec![mt],
            },
            Instruction {
                op: Op::BeginFunction { param_count: 2 },
                inputs: vec![],
                outputs: vec![index_fn_var, index_self, index_key],
            },
            Instruction {
                op: index_body_op,
                inputs: vec![],
                outputs: vec![result],
            },
            Instruction {
                op: Op::Return,
                inputs: vec![result],
                outputs: vec![],
            },
            Instruction {
                op: Op::EndFunction,
                inputs: vec![],
                outputs: vec![],
            },
            Instruction {
                op: Op::TableSetField("__index".into()),
                inputs: vec![mt, index_fn_var],
                outputs: vec![],
            },
            Instruction {
                op: Op::SetMetatable,
                inputs: vec![env_table, mt],
                outputs: vec![mt_result],
            },
            Instruction {
                op: Op::SetFenv,
                inputs: vec![fn_var, env_table],
                outputs: vec![],
            },
        ];

        let tail = program.instructions.split_off(fn_end);
        program.instructions.extend(new_instrs);
        program.instructions.extend(tail);
        true
    }

    fn swap_function_env(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let fn_blocks = find_fn_blocks(program);
        if fn_blocks.len() < 2 {
            return false;
        }

        let a = rng.random_range(0..fn_blocks.len());
        let mut b = rng.random_range(0..fn_blocks.len() - 1);
        if b >= a {
            b += 1;
        }

        let env_var = program.new_var();

        let get_env = Instruction {
            op: Op::GetFenv,
            inputs: vec![fn_blocks[a].2],
            outputs: vec![env_var],
        };
        let set_env = Instruction {
            op: Op::SetFenv,
            inputs: vec![fn_blocks[b].2, env_var],
            outputs: vec![],
        };

        program.instructions.push(get_env);
        program.instructions.push(set_env);
        true
    }

    fn setfenv_raw(&self, program: &mut Program, rng: &mut dyn RngCore) -> bool {
        let fn_blocks = find_fn_blocks(program);
        if fn_blocks.is_empty() {
            return false;
        }

        let &(_, fn_end, fn_var) = pick_random(&fn_blocks, rng);

        let table_candidates: Vec<Variable> =
            find_compatible_variables(program, fn_end, Some(LuaType::Table))
                .into_iter()
                .map(|sv| sv.var)
                .collect();

        let env = if rng.random_bool(0.5) && !table_candidates.is_empty() {
            table_candidates[rng.random_range(0..table_candidates.len())]
        } else {
            let v = program.new_var();
            program.instructions.insert(
                fn_end,
                Instruction {
                    op: Op::CreateTable,
                    inputs: vec![],
                    outputs: vec![v],
                },
            );
            v
        };

        program.instructions.push(Instruction {
            op: Op::SetFenv,
            inputs: vec![fn_var, env],
            outputs: vec![],
        });
        true
    }
}
