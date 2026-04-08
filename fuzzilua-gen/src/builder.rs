use fuzzilua_ir::{ArityCount, BlockKind, LuaType, Op, Program, Variable};
use rand::Rng;
use rand::RngCore;

const DEFAULT_MAX_DEPTH: usize = 5;
const DEFAULT_BUDGET: usize = 200;

pub struct ProgramBuilder {
    program: Program,
    scope_stack: Vec<Vec<(Variable, LuaType)>>,
    block_kinds: Vec<BlockKind>,
    budget: usize,
    max_depth: usize,
}

impl ProgramBuilder {
    pub fn new(budget: usize, max_depth: usize) -> Self {
        Self {
            program: Program::new(),
            scope_stack: vec![Vec::new()],
            block_kinds: Vec::new(),
            budget,
            max_depth,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_BUDGET, DEFAULT_MAX_DEPTH)
    }

    pub fn with_scope(
        budget: usize,
        max_depth: usize,
        next_var: u32,
        visible: Vec<(Variable, LuaType)>,
    ) -> Self {
        let mut program = Program::new();
        program.next_var = next_var;
        Self {
            program,
            scope_stack: vec![visible],
            block_kinds: Vec::new(),
            budget,
            max_depth,
        }
    }

    pub fn remaining_budget(&self) -> usize {
        let reserved = self.block_kinds.len();
        self.budget
            .saturating_sub(self.program.instructions.len())
            .saturating_sub(reserved)
    }

    pub fn current_depth(&self) -> usize {
        self.block_kinds.len()
    }

    pub fn emit(&mut self, op: Op, inputs: Vec<Variable>) -> Option<Vec<Variable>> {
        if self.remaining_budget() == 0 {
            return None;
        }
        let out_count = match op.arity().outputs {
            ArityCount::Exact(n) => n,
            ArityCount::AtLeast(n) => n,
        };
        self.emit_n(op, inputs, out_count)
    }

    pub fn emit_n(
        &mut self,
        op: Op,
        inputs: Vec<Variable>,
        output_count: usize,
    ) -> Option<Vec<Variable>> {
        if self.remaining_budget() == 0 {
            return None;
        }
        let lua_type = Program::infer_output_type(&op);
        let outputs: Vec<Variable> = (0..output_count).map(|_| self.program.new_var()).collect();
        self.program.emit(op, inputs, outputs.clone());
        if let Some(scope) = self.scope_stack.last_mut() {
            for &v in &outputs {
                scope.push((v, lua_type));
            }
        }
        Some(outputs)
    }

    pub fn begin_block(&mut self, op: Op, inputs: Vec<Variable>) -> Option<Vec<Variable>> {
        let block_kind = op.opens_block()?;
        if block_kind == BlockKind::Else {
            return None;
        }
        if self.current_depth() >= self.max_depth || self.remaining_budget() < 2 {
            return None;
        }
        let outputs = self.emit(op, inputs)?;
        self.block_kinds.push(block_kind);
        self.scope_stack.push(Vec::new());
        if !outputs.is_empty() {
            let lua_type = self
                .scope_stack
                .iter()
                .rev()
                .skip(1)
                .flat_map(|s| s.iter())
                .find(|(v, _)| *v == outputs[0])
                .map(|(_, t)| *t)
                .unwrap_or(LuaType::Anything);
            if let Some(parent) = self.scope_stack.iter_mut().rev().nth(1) {
                parent.retain(|(v, _)| !outputs.contains(v));
            }
            let child = self.scope_stack.last_mut().unwrap();
            for &v in &outputs {
                child.push((v, lua_type));
            }
        }
        Some(outputs)
    }

    pub fn begin_else(&mut self) -> bool {
        if !matches!(self.block_kinds.last(), Some(BlockKind::If)) {
            return false;
        }
        if self.remaining_budget() == 0 {
            return false;
        }
        self.scope_stack.pop();
        self.scope_stack.push(Vec::new());
        *self.block_kinds.last_mut().unwrap() = BlockKind::Else;
        self.program.emit(Op::BeginElse, vec![], vec![]);
        true
    }

    pub fn end_block(&mut self) -> bool {
        let Some(kind) = self.block_kinds.pop() else {
            return false;
        };
        self.scope_stack.pop();
        let close_op = closing_op(kind);
        self.program.emit(close_op, vec![], vec![]);
        true
    }

    pub fn in_loop(&self) -> bool {
        self.block_kinds
            .iter()
            .any(|b| matches!(b, BlockKind::While | BlockKind::ForIn | BlockKind::ForRange))
    }

    pub fn visible_variables(&self) -> impl Iterator<Item = (Variable, LuaType)> + '_ {
        self.scope_stack.iter().flat_map(|s| s.iter().copied())
    }

    pub fn random_variable_of_type(&self, ty: LuaType, rng: &mut dyn RngCore) -> Option<Variable> {
        let candidates: Vec<Variable> = self
            .visible_variables()
            .filter(|(_, t)| *t == ty || *t == LuaType::Anything || ty == LuaType::Anything)
            .map(|(v, _)| v)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates[rng.random_range(0..candidates.len())])
    }

    pub fn ensure_table(&mut self, rng: &mut dyn RngCore) -> Option<Variable> {
        if let Some(v) = self.random_variable_of_type(LuaType::Table, rng) {
            return Some(v);
        }
        self.emit(Op::CreateTable, vec![]).map(|v| v[0])
    }

    pub fn ensure_string(&mut self, rng: &mut dyn RngCore) -> Option<Variable> {
        if let Some(v) = self.random_variable_of_type(LuaType::String, rng) {
            return Some(v);
        }
        self.emit(Op::LoadString(random_string(rng).into()), vec![])
            .map(|v| v[0])
    }

    pub fn ensure_number(&mut self, rng: &mut dyn RngCore) -> Option<Variable> {
        if let Some(v) = self.random_variable_of_type(LuaType::Number, rng) {
            return Some(v);
        }
        if let Some(v) = self.random_variable_of_type(LuaType::Integer, rng) {
            return Some(v);
        }
        self.emit(Op::LoadInt(rng.random_range(-100..100)), vec![])
            .map(|v| v[0])
    }

    pub fn ensure_function(&mut self, rng: &mut dyn RngCore) -> Option<Variable> {
        if let Some(v) = self.random_variable_of_type(LuaType::Function, rng) {
            return Some(v);
        }
        let param_count = rng.random_range(0..=2) as u32;
        let outputs = self.begin_block(Op::BeginFunction { param_count }, vec![])?;
        if let Some(v) = self.ensure_number(rng) {
            self.emit(Op::Return, vec![v]);
        }
        self.end_block();
        Some(outputs[0])
    }

    pub fn ensure_any(&mut self, rng: &mut dyn RngCore) -> Option<Variable> {
        self.random_variable_of_type(LuaType::Anything, rng)
            .or_else(|| {
                self.emit(Op::LoadInt(rng.random_range(0..100)), vec![])
                    .map(|v| v[0])
            })
    }

    pub fn finish(mut self) -> Program {
        while !self.block_kinds.is_empty() {
            self.end_block();
        }
        self.program
    }
}

fn closing_op(kind: BlockKind) -> Op {
    match kind {
        BlockKind::If | BlockKind::Else => Op::EndIf,
        BlockKind::While => Op::EndWhile,
        BlockKind::ForIn => Op::EndForIn,
        BlockKind::ForRange => Op::EndForRange,
        BlockKind::Function => Op::EndFunction,
        BlockKind::Pcall => Op::EndPcall,
    }
}

pub fn random_string(rng: &mut dyn RngCore) -> String {
    let len = rng.random_range(1..=8);
    (0..len)
        .map(|_| rng.random_range(b'a'..=b'z') as char)
        .collect()
}

pub fn random_field_name(rng: &mut dyn RngCore) -> String {
    const NAMES: &[&str] = &[
        "x", "y", "key", "val", "n", "data", "meta", "idx", "tmp", "buf",
    ];
    NAMES[rng.random_range(0..NAMES.len())].to_string()
}
