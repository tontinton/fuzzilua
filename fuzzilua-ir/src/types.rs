use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Variable(pub u32);

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LuaType {
    Nil,
    Boolean,
    Integer,
    Number,
    String,
    Table,
    Function,
    Coroutine,
    Userdata,
    Anything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add => write!(f, "+"),
            Self::Sub => write!(f, "-"),
            Self::Mul => write!(f, "*"),
            Self::Div => write!(f, "/"),
            Self::Mod => write!(f, "%"),
            Self::Pow => write!(f, "^"),
            Self::Concat => write!(f, ".."),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnOp {
    Neg,
    Not,
    Len,
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Neg => write!(f, "-"),
            Self::Not => write!(f, "not "),
            Self::Len => write!(f, "#"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eq => write!(f, "=="),
            Self::Ne => write!(f, "~="),
            Self::Lt => write!(f, "<"),
            Self::Le => write!(f, "<="),
            Self::Gt => write!(f, ">"),
            Self::Ge => write!(f, ">="),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GcMode {
    Collect,
    Stop,
    Restart,
    Step,
}

impl fmt::Display for GcMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collect => write!(f, "\"collect\""),
            Self::Stop => write!(f, "\"stop\""),
            Self::Restart => write!(f, "\"restart\""),
            Self::Step => write!(f, "\"step\""),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metamethod {
    pub name: &'static str,
    pub param_count: u32,
}

pub const METAMETHODS: &[Metamethod] = &[
    Metamethod {
        name: "__index",
        param_count: 2,
    },
    Metamethod {
        name: "__newindex",
        param_count: 2,
    },
    Metamethod {
        name: "__eq",
        param_count: 2,
    },
    Metamethod {
        name: "__concat",
        param_count: 2,
    },
    Metamethod {
        name: "__len",
        param_count: 1,
    },
    Metamethod {
        name: "__add",
        param_count: 2,
    },
    Metamethod {
        name: "__sub",
        param_count: 2,
    },
    Metamethod {
        name: "__mul",
        param_count: 2,
    },
    Metamethod {
        name: "__div",
        param_count: 2,
    },
    Metamethod {
        name: "__mod",
        param_count: 2,
    },
    Metamethod {
        name: "__pow",
        param_count: 2,
    },
    Metamethod {
        name: "__unm",
        param_count: 1,
    },
    Metamethod {
        name: "__lt",
        param_count: 2,
    },
    Metamethod {
        name: "__le",
        param_count: 2,
    },
    Metamethod {
        name: "__call",
        param_count: 1,
    },
    Metamethod {
        name: "__tostring",
        param_count: 1,
    },
    Metamethod {
        name: "__gc",
        param_count: 1,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlockKind {
    If,
    Else,
    While,
    ForIn,
    ForRange,
    Function,
    Pcall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arity {
    pub inputs: ArityCount,
    pub outputs: ArityCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArityCount {
    Exact(usize),
    AtLeast(usize),
}

impl ArityCount {
    pub fn accepts(&self, n: usize) -> bool {
        match self {
            Self::Exact(expected) => n == *expected,
            Self::AtLeast(min) => n >= *min,
        }
    }
}

// String fields use Arc<str> so Program::clone() is near-free for string data
// (ref-count bump instead of heap allocation per string). Cloning programs is
// the hot path in the mutation engine's snapshot/rollback loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    LoadNil,
    LoadBool(bool),
    LoadInt(i64),
    LoadFloat(f64),
    LoadString(Arc<str>),

    Reassign,

    CreateTable,
    TableSetField(Arc<str>),
    TableGetField(Arc<str>),
    TableSetIndex,
    TableGetIndex,
    TableSetNumericField(i64),
    TableGetNumericField(i64),

    BeginFunction { param_count: u32 },
    EndFunction,
    Return,
    CallFunction { arg_count: u32, ret_count: u32 },

    BeginIf,
    BeginElse,
    EndIf,
    BeginWhile,
    EndWhile,
    BeginForIn,
    EndForIn,
    BeginForRange,
    EndForRange,
    Break,

    BinaryOp(BinOp),
    UnaryOp(UnOp),
    Compare(CmpOp),

    SetMetatable,
    GetMetatable,

    TypeOf,
    ToNumber,
    ToString,
    ToStringFmt(Arc<str>),
    Print,
    RawGet,
    RawSet,
    RawEqual,
    Select,
    Unpack,
    Ipairs,
    Pairs,
    Next,
    SetFenv,
    GetFenv,

    CollectGarbage(GcMode),

    CoroutineCreate,
    CoroutineResume,
    CoroutineYield,
    CoroutineWrap,

    BeginPcall,
    EndPcall,

    StringLen,
    StringSub,
    StringFind,
    StringFormat(Arc<str>),
    StringRep,
    StringByte,
    StringChar,
    StringGmatch(Arc<str>),
    StringGsub(Arc<str>, Arc<str>),

    Loadstring,

    Nop,
}

use ArityCount::{AtLeast, Exact};

impl Op {
    pub fn arity(&self) -> Arity {
        let (inputs, outputs) = match self {
            Self::LoadNil => (Exact(0), Exact(1)),
            Self::LoadBool(_) => (Exact(0), Exact(1)),
            Self::LoadInt(_) => (Exact(0), Exact(1)),
            Self::LoadFloat(_) => (Exact(0), Exact(1)),
            Self::LoadString(_) => (Exact(0), Exact(1)),

            Self::Reassign => (Exact(1), Exact(1)),

            Self::CreateTable => (Exact(0), Exact(1)),
            Self::TableSetField(_) => (Exact(2), Exact(0)),
            Self::TableGetField(_) => (Exact(1), Exact(1)),
            Self::TableSetIndex => (Exact(3), Exact(0)),
            Self::TableGetIndex => (Exact(2), Exact(1)),
            Self::TableSetNumericField(_) => (Exact(2), Exact(0)),
            Self::TableGetNumericField(_) => (Exact(1), Exact(1)),

            Self::BeginFunction { param_count } => (Exact(0), Exact(1 + *param_count as usize)),
            Self::EndFunction => (Exact(0), Exact(0)),
            Self::Return => (AtLeast(0), Exact(0)),
            Self::CallFunction {
                arg_count,
                ret_count,
            } => (Exact(1 + *arg_count as usize), Exact(*ret_count as usize)),

            Self::BeginIf => (Exact(1), Exact(0)),
            Self::BeginElse => (Exact(0), Exact(0)),
            Self::EndIf => (Exact(0), Exact(0)),
            Self::BeginWhile => (Exact(1), Exact(0)),
            Self::EndWhile => (Exact(0), Exact(0)),
            Self::BeginForIn => (AtLeast(1), AtLeast(1)),
            Self::EndForIn => (Exact(0), Exact(0)),
            Self::BeginForRange => (AtLeast(2), Exact(1)),
            Self::EndForRange => (Exact(0), Exact(0)),
            Self::Break => (Exact(0), Exact(0)),

            Self::BinaryOp(_) => (Exact(2), Exact(1)),
            Self::UnaryOp(_) => (Exact(1), Exact(1)),
            Self::Compare(_) => (Exact(2), Exact(1)),

            Self::SetMetatable => (Exact(2), Exact(1)),
            Self::GetMetatable => (Exact(1), Exact(1)),

            Self::TypeOf => (Exact(1), Exact(1)),
            Self::ToNumber => (Exact(1), Exact(1)),
            Self::ToString => (Exact(1), Exact(1)),
            Self::ToStringFmt(_) => (Exact(1), Exact(1)),
            Self::Print => (AtLeast(1), Exact(0)),
            Self::RawGet => (Exact(2), Exact(1)),
            Self::RawSet => (Exact(3), Exact(0)),
            Self::RawEqual => (Exact(2), Exact(1)),
            Self::Select => (Exact(2), Exact(1)),
            Self::Unpack => (Exact(1), Exact(1)),
            Self::Ipairs => (Exact(1), Exact(1)),
            Self::Pairs => (Exact(1), Exact(1)),
            Self::Next => (AtLeast(1), AtLeast(1)),
            Self::SetFenv => (Exact(2), Exact(0)),
            Self::GetFenv => (Exact(1), Exact(1)),

            Self::CollectGarbage(_) => (Exact(0), Exact(0)),

            Self::CoroutineCreate => (Exact(1), Exact(1)),
            Self::CoroutineResume => (AtLeast(1), AtLeast(0)),
            Self::CoroutineYield => (AtLeast(0), Exact(0)),
            Self::CoroutineWrap => (Exact(1), Exact(1)),

            Self::BeginPcall => (Exact(0), Exact(1)),
            Self::EndPcall => (Exact(0), Exact(0)),

            Self::StringLen => (Exact(1), Exact(1)),
            Self::StringSub => (AtLeast(2), Exact(1)),
            Self::StringFind => (Exact(2), Exact(1)),
            Self::StringFormat(_) => (AtLeast(0), Exact(1)),
            Self::StringRep => (Exact(2), Exact(1)),
            Self::StringByte => (Exact(1), Exact(1)),
            Self::StringChar => (Exact(1), Exact(1)),
            Self::StringGmatch(_) => (Exact(1), Exact(1)),
            Self::StringGsub(_, _) => (Exact(1), Exact(1)),

            Self::Loadstring => (Exact(1), Exact(1)),

            Self::Nop => (Exact(0), Exact(0)),
        };
        Arity { inputs, outputs }
    }

    pub fn opens_block(&self) -> Option<BlockKind> {
        match self {
            Self::BeginIf => Some(BlockKind::If),
            Self::BeginElse => Some(BlockKind::Else),
            Self::BeginWhile => Some(BlockKind::While),
            Self::BeginForIn => Some(BlockKind::ForIn),
            Self::BeginForRange => Some(BlockKind::ForRange),
            Self::BeginFunction { .. } => Some(BlockKind::Function),
            Self::BeginPcall => Some(BlockKind::Pcall),
            _ => None,
        }
    }

    pub fn closes_block(&self) -> Option<BlockKind> {
        match self {
            Self::EndIf => Some(BlockKind::If),
            Self::EndWhile => Some(BlockKind::While),
            Self::EndForIn => Some(BlockKind::ForIn),
            Self::EndForRange => Some(BlockKind::ForRange),
            Self::EndFunction => Some(BlockKind::Function),
            Self::EndPcall => Some(BlockKind::Pcall),
            _ => None,
        }
    }

    /// Whether this op has side effects beyond defining its outputs.
    /// Used by dead-variable elimination in minimize.
    pub fn is_effectful(&self) -> bool {
        matches!(
            self,
            Self::TableSetField(_)
                | Self::TableSetIndex
                | Self::TableSetNumericField(_)
                | Self::SetMetatable
                | Self::Print
                | Self::RawSet
                | Self::SetFenv
                | Self::CollectGarbage(_)
                | Self::CallFunction { .. }
                | Self::Return
                | Self::Break
                | Self::BeginFunction { .. }
                | Self::EndFunction
                | Self::BeginIf
                | Self::BeginElse
                | Self::EndIf
                | Self::BeginWhile
                | Self::EndWhile
                | Self::BeginForIn
                | Self::EndForIn
                | Self::BeginForRange
                | Self::EndForRange
                | Self::BeginPcall
                | Self::EndPcall
                | Self::CoroutineResume
                | Self::CoroutineYield
        )
    }
}

#[derive(Debug, Clone, Error)]
#[error("{op:?}: expected {expected:?} {kind}, got {actual}")]
pub struct ArityError {
    pub op: Op,
    pub kind: &'static str,
    pub expected: ArityCount,
    pub actual: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instruction {
    pub op: Op,
    pub inputs: Vec<Variable>,
    pub outputs: Vec<Variable>,
}

impl Instruction {
    pub fn nop() -> Self {
        Self {
            op: Op::Nop,
            inputs: vec![],
            outputs: vec![],
        }
    }

    pub fn new(op: Op, inputs: Vec<Variable>, outputs: Vec<Variable>) -> Result<Self, ArityError> {
        let arity = op.arity();
        if !arity.inputs.accepts(inputs.len()) {
            return Err(ArityError {
                op,
                kind: "inputs",
                expected: arity.inputs,
                actual: inputs.len(),
            });
        }
        if !arity.outputs.accepts(outputs.len()) {
            return Err(ArityError {
                op,
                kind: "outputs",
                expected: arity.outputs,
                actual: outputs.len(),
            });
        }
        Ok(Self {
            op,
            inputs,
            outputs,
        })
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.outputs.is_empty() {
            let outs: Vec<String> = self.outputs.iter().map(|v| v.to_string()).collect();
            write!(f, "{} = ", outs.join(", "))?;
        }
        write!(f, "{:?}", self.op)?;
        if !self.inputs.is_empty() {
            let ins: Vec<String> = self.inputs.iter().map(|v| v.to_string()).collect();
            write!(f, " {}", ins.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub instructions: Vec<Instruction>,
    pub next_var: u32,
}

impl Program {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            next_var: 0,
        }
    }

    pub fn new_var(&mut self) -> Variable {
        let v = Variable(self.next_var);
        self.next_var += 1;
        v
    }

    pub fn emit(&mut self, op: Op, inputs: Vec<Variable>, outputs: Vec<Variable>) {
        let instr = Instruction::new(op, inputs, outputs).expect("arity mismatch in Program::emit");
        self.instructions.push(instr);
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}
