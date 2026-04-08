//! Intermediate representation for generated Lua programs.
//!
//! - `Op::arity()` is the source of truth for input/output counts.
//!   `Instruction::new()` validates arity at construction.
//! - `Op::opens_block()` / `Op::closes_block()` is the source of truth for
//!   block structure. `BlockKind` is shared across validate, scope, lift,
//!   and the gen crate, so adding a new block kind works everywhere.
//! - `Instruction` fields are pub for serde, but prefer `Instruction::new()`
//!   for arity validation.
//! - `CallFunction` and `BeginFunction` keep explicit counts because arity
//!   validation depends on them.

mod bitset;
mod lift;
mod scope;
#[cfg(test)]
mod tests;
mod types;
mod validate;

pub use bitset::VarBitset;
pub use lift::lift;
pub use scope::ScopedVariable;
pub use types::{
    Arity, ArityCount, ArityError, BinOp, BlockKind, CmpOp, GcMode, Instruction, LuaType,
    METAMETHODS, Metamethod, Op, Program, UnOp, Variable,
};
pub use validate::ValidationError;
