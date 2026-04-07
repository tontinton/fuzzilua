//! Intermediate representation for generated Lua programs.
//!
//! Key design choices:
//!
//! - `Op::arity()` is the single source of truth for input/output counts.
//!   `Instruction::new()` validates arity at construction; `validate()` re-checks
//!   for deserialized programs that bypass the constructor.
//! - `Op::opens_block()` / `Op::closes_block()` is the single source of truth for
//!   block structure. `BlockKind` (in types.rs) is shared by validate, scope, lift,
//!   and the gen crate's ProgramBuilder, so adding a new block kind just works everywhere.
//! - `Instruction` fields are pub for serde, but construction should go through
//!   `Instruction::new()` to get arity validation.
//! - `CallFunction { arg_count, ret_count }` and `BeginFunction { param_count }` keep
//!   explicit counts because they enable strict arity validation that would be lost
//!   if we tried to derive them.
//! - validate.rs helper fns (`check_arity`, `check_inputs_defined`, `register_outputs`)
//!   are associated fns (no `&self`) since they don't touch Program fields.
//! - `Program::infer_output_type` is pub because the gen crate's ProgramBuilder needs it.

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
