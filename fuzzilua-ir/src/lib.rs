mod lift;
mod scope;
#[cfg(test)]
mod tests;
mod types;
mod validate;

pub use lift::lift;
pub use scope::ScopedVariable;
pub use types::{
    Arity, ArityCount, ArityError, BinOp, BlockKind, CmpOp, GcMode, Instruction, LuaType, Op,
    Program, UnOp, Variable,
};
pub use validate::ValidationError;
