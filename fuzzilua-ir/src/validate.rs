use crate::types::{BlockKind, Instruction, Op, Program, Variable};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub index: usize,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "instruction {}: {}", self.index, self.message)
    }
}

impl std::error::Error for ValidationError {}

impl BlockKind {
    fn closes_with(&self) -> &[BlockKind] {
        match self {
            Self::If => &[BlockKind::If, BlockKind::Else],
            Self::Else => &[BlockKind::If],
            Self::While => &[BlockKind::While],
            Self::ForIn => &[BlockKind::ForIn],
            Self::ForRange => &[BlockKind::ForRange],
            Self::Function => &[BlockKind::Function],
            Self::Pcall => &[BlockKind::Pcall],
        }
    }

    fn is_loop(&self) -> bool {
        matches!(self, Self::While | Self::ForIn | Self::ForRange)
    }
}

impl Program {
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let mut defined: HashSet<Variable> = HashSet::new();
        let mut block_stack: Vec<(BlockKind, usize)> = Vec::new();
        let mut max_var: Option<u32> = None;

        for (i, instr) in self.instructions.iter().enumerate() {
            Self::check_arity(i, instr, &mut errors);
            Self::check_inputs_defined(i, instr, &defined, &mut errors);
            Self::register_outputs(instr, &mut defined, &mut max_var);
            Self::check_block_structure(i, instr, &mut block_stack, &mut errors);
        }

        for (kind, open_idx) in &block_stack {
            errors.push(ValidationError {
                index: *open_idx,
                message: format!("unclosed {kind:?} block"),
            });
        }

        if let Some(max) = max_var
            && self.next_var <= max
        {
            errors.push(ValidationError {
                index: 0,
                message: format!(
                    "next_var ({}) must be > max variable id ({max})",
                    self.next_var
                ),
            });
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn check_arity(index: usize, instr: &Instruction, errors: &mut Vec<ValidationError>) {
        let arity = instr.op.arity();
        if !arity.inputs.accepts(instr.inputs.len()) {
            errors.push(ValidationError {
                index,
                message: format!(
                    "{:?}: expected {:?} inputs, got {}",
                    instr.op,
                    arity.inputs,
                    instr.inputs.len()
                ),
            });
        }
        if !arity.outputs.accepts(instr.outputs.len()) {
            errors.push(ValidationError {
                index,
                message: format!(
                    "{:?}: expected {:?} outputs, got {}",
                    instr.op,
                    arity.outputs,
                    instr.outputs.len()
                ),
            });
        }
    }

    fn check_inputs_defined(
        index: usize,
        instr: &Instruction,
        defined: &HashSet<Variable>,
        errors: &mut Vec<ValidationError>,
    ) {
        for v in &instr.inputs {
            if !defined.contains(v) {
                errors.push(ValidationError {
                    index,
                    message: format!("variable {v} used before definition"),
                });
            }
        }
    }

    fn register_outputs(
        instr: &Instruction,
        defined: &mut HashSet<Variable>,
        max_var: &mut Option<u32>,
    ) {
        for v in &instr.outputs {
            defined.insert(*v);
            *max_var = Some(max_var.map_or(v.0, |m: u32| m.max(v.0)));
        }
    }

    fn check_block_structure(
        index: usize,
        instr: &Instruction,
        block_stack: &mut Vec<(BlockKind, usize)>,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(opens) = instr.op.opens_block() {
            if opens == BlockKind::Else {
                if block_stack.last().map(|(k, _)| *k) != Some(BlockKind::If) {
                    errors.push(ValidationError {
                        index,
                        message: "BeginElse without matching BeginIf".to_string(),
                    });
                } else {
                    block_stack.pop();
                    block_stack.push((BlockKind::Else, index));
                }
            } else {
                block_stack.push((opens, index));
            }
        } else if let Some(closes) = instr.op.closes_block() {
            let top = block_stack.last().map(|(k, _)| *k);
            let expected = closes.closes_with();
            if top.is_some_and(|k| expected.contains(&k)) {
                block_stack.pop();
            } else {
                errors.push(ValidationError {
                    index,
                    message: format!("{:?} end without matching {:?}", closes, expected),
                });
            }
        } else if matches!(instr.op, Op::Break) {
            let in_loop = block_stack.iter().rev().any(|(k, _)| k.is_loop());
            if !in_loop {
                errors.push(ValidationError {
                    index,
                    message: "Break outside of loop".to_string(),
                });
            }
        }
    }
}
