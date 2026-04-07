use crate::types::{LuaType, Op, Program, Variable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedVariable {
    pub var: Variable,
    pub lua_type: LuaType,
    pub depth: usize,
}

impl Program {
    pub fn variables_in_scope_at(&self, index: usize) -> Vec<ScopedVariable> {
        let mut scope_stack: Vec<Vec<ScopedVariable>> = vec![Vec::new()];

        for (i, instr) in self.instructions.iter().enumerate() {
            if i == index {
                return scope_stack.into_iter().flatten().collect();
            }

            if instr.op.closes_block().is_some() {
                scope_stack.pop();
            } else {
                if instr.op.opens_block().is_some() {
                    scope_stack.push(Vec::new());
                }
                let depth = scope_stack.len() - 1;
                let lua_type = Self::infer_output_type(&instr.op);
                for v in &instr.outputs {
                    scope_stack.last_mut().unwrap().push(ScopedVariable {
                        var: *v,
                        lua_type,
                        depth,
                    });
                }
            }
        }

        scope_stack.into_iter().flatten().collect()
    }

    fn infer_output_type(op: &Op) -> LuaType {
        match op {
            Op::LoadNil => LuaType::Nil,
            Op::LoadBool(_) | Op::Compare(_) | Op::UnaryOp(crate::types::UnOp::Not) => {
                LuaType::Boolean
            }
            Op::LoadInt(_)
            | Op::StringLen
            | Op::StringByte
            | Op::UnaryOp(crate::types::UnOp::Len) => LuaType::Integer,
            Op::LoadFloat(_) | Op::ToNumber => LuaType::Number,
            Op::LoadString(_)
            | Op::TypeOf
            | Op::ToString
            | Op::ToStringFmt(_)
            | Op::StringSub
            | Op::StringChar
            | Op::StringFormat(_)
            | Op::StringRep
            | Op::StringGsub(_, _) => LuaType::String,
            Op::CreateTable | Op::GetMetatable => LuaType::Table,
            Op::BeginFunction { .. }
            | Op::StringGmatch(_)
            | Op::Loadstring
            | Op::Ipairs
            | Op::Pairs
            | Op::Next => LuaType::Function,
            Op::CoroutineCreate | Op::CoroutineWrap => LuaType::Coroutine,
            _ => LuaType::Anything,
        }
    }
}
