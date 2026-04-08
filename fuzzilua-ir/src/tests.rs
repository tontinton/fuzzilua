use crate::{BinOp, CmpOp, GcMode, Instruction, Op, Program, UnOp, Variable, lift};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const FUZZ_SEED: u64 = 12345;
const FUZZ_ITERATIONS: usize = 10_000;

fn random_program(rng: &mut StdRng, max_instrs: usize) -> Program {
    let mut prog = Program::new();
    let num_instrs = rng.random_range(1..=max_instrs);
    let mut block_stack: Vec<RandBlock> = Vec::new();

    for _ in 0..num_instrs {
        let available: Vec<Variable> = (0..prog.next_var).map(Variable).collect();

        if let Some(kind) = block_stack.last() {
            if rng.random_range(0..10) < 3 {
                close_block(&mut prog, &mut block_stack);
                continue;
            }
            if matches!(kind, RandBlock::If) && rng.random_range(0..10) < 2 {
                prog.emit(Op::BeginElse, vec![], vec![]);
                *block_stack.last_mut().unwrap() = RandBlock::Else;
                continue;
            }
        }

        if rng.random_range(0..10) < 2 && block_stack.len() < 5 {
            open_random_block(rng, &mut prog, &available, &mut block_stack);
            continue;
        }

        emit_random_flat_op(rng, &mut prog, &available);
    }

    while !block_stack.is_empty() {
        close_block(&mut prog, &mut block_stack);
    }

    prog
}

#[derive(Clone, Copy)]
enum RandBlock {
    If,
    Else,
    While,
    ForRange,
    Function,
}

fn open_random_block(
    rng: &mut StdRng,
    prog: &mut Program,
    available: &[Variable],
    stack: &mut Vec<RandBlock>,
) {
    match rng.random_range(0..4) {
        0 => {
            let b: bool = rng.random();
            let cond = pick_or_create(rng, prog, available, Op::LoadBool(b));
            prog.emit(Op::BeginIf, vec![cond], vec![]);
            stack.push(RandBlock::If);
        }
        1 => {
            let b: bool = rng.random();
            let cond = pick_or_create(rng, prog, available, Op::LoadBool(b));
            prog.emit(Op::BeginWhile, vec![cond], vec![]);
            stack.push(RandBlock::While);
        }
        2 => {
            let s: i64 = rng.random_range(-100..100);
            let e: i64 = rng.random_range(-100..100);
            let start = pick_or_create(rng, prog, available, Op::LoadInt(s));
            let stop = pick_or_create(rng, prog, available, Op::LoadInt(e));
            let iter_var = prog.new_var();
            prog.emit(Op::BeginForRange, vec![start, stop], vec![iter_var]);
            stack.push(RandBlock::ForRange);
        }
        _ => {
            let param_count = rng.random_range(0..3u32);
            let func_var = prog.new_var();
            let mut outputs = vec![func_var];
            for _ in 0..param_count {
                outputs.push(prog.new_var());
            }
            prog.emit(Op::BeginFunction { param_count }, vec![], outputs);
            stack.push(RandBlock::Function);
        }
    }
}

fn close_block(prog: &mut Program, stack: &mut Vec<RandBlock>) {
    if let Some(kind) = stack.pop() {
        let end_op = match kind {
            RandBlock::If | RandBlock::Else => Op::EndIf,
            RandBlock::While => Op::EndWhile,
            RandBlock::ForRange => Op::EndForRange,
            RandBlock::Function => Op::EndFunction,
        };
        prog.emit(end_op, vec![], vec![]);
    }
}

fn pick_or_create(
    rng: &mut StdRng,
    prog: &mut Program,
    available: &[Variable],
    fallback: Op,
) -> Variable {
    if !available.is_empty() && rng.random_range(0..3) > 0 {
        available[rng.random_range(0..available.len())]
    } else {
        let v = prog.new_var();
        prog.emit(fallback, vec![], vec![v]);
        v
    }
}

fn pick_var(rng: &mut StdRng, available: &[Variable]) -> Option<Variable> {
    if available.is_empty() {
        None
    } else {
        Some(available[rng.random_range(0..available.len())])
    }
}

fn emit_random_flat_op(rng: &mut StdRng, prog: &mut Program, available: &[Variable]) {
    match rng.random_range(0..20) {
        0 => {
            let v = prog.new_var();
            prog.emit(Op::LoadNil, vec![], vec![v]);
        }
        1 => {
            let v = prog.new_var();
            prog.emit(Op::LoadBool(rng.random()), vec![], vec![v]);
        }
        2 => {
            let v = prog.new_var();
            prog.emit(Op::LoadInt(rng.random_range(-1000..1000)), vec![], vec![v]);
        }
        3 => {
            let v = prog.new_var();
            prog.emit(
                Op::LoadFloat(rng.random_range(-100.0..100.0)),
                vec![],
                vec![v],
            );
        }
        4 => {
            let v = prog.new_var();
            let s = random_string(rng);
            prog.emit(Op::LoadString(s.into()), vec![], vec![v]);
        }
        5 => {
            let v = prog.new_var();
            prog.emit(Op::CreateTable, vec![], vec![v]);
        }
        6 => {
            if let (Some(a), Some(b)) = (pick_var(rng, available), pick_var(rng, available)) {
                let v = prog.new_var();
                let ops = [
                    BinOp::Add,
                    BinOp::Sub,
                    BinOp::Mul,
                    BinOp::Div,
                    BinOp::Mod,
                    BinOp::Pow,
                    BinOp::Concat,
                ];
                let op = ops[rng.random_range(0..ops.len())];
                prog.emit(Op::BinaryOp(op), vec![a, b], vec![v]);
            }
        }
        7 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                let ops = [UnOp::Neg, UnOp::Not, UnOp::Len];
                let op = ops[rng.random_range(0..ops.len())];
                prog.emit(Op::UnaryOp(op), vec![a], vec![v]);
            }
        }
        8 => {
            if let (Some(a), Some(b)) = (pick_var(rng, available), pick_var(rng, available)) {
                let v = prog.new_var();
                let ops = [
                    CmpOp::Eq,
                    CmpOp::Ne,
                    CmpOp::Lt,
                    CmpOp::Le,
                    CmpOp::Gt,
                    CmpOp::Ge,
                ];
                let op = ops[rng.random_range(0..ops.len())];
                prog.emit(Op::Compare(op), vec![a, b], vec![v]);
            }
        }
        9 => {
            if let Some(t) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::TableGetField("key".into()), vec![t], vec![v]);
            }
        }
        10 => {
            if let (Some(t), Some(val)) = (pick_var(rng, available), pick_var(rng, available)) {
                prog.emit(Op::TableSetField("key".into()), vec![t, val], vec![]);
            }
        }
        11 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::TypeOf, vec![a], vec![v]);
            }
        }
        12 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::ToString, vec![a], vec![v]);
            }
        }
        13 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::ToNumber, vec![a], vec![v]);
            }
        }
        14 => {
            let modes = [GcMode::Collect, GcMode::Stop, GcMode::Restart, GcMode::Step];
            let mode = modes[rng.random_range(0..modes.len())];
            prog.emit(Op::CollectGarbage(mode), vec![], vec![]);
        }
        15 => {
            if let Some(a) = pick_var(rng, available) {
                prog.emit(Op::Print, vec![a], vec![]);
            }
        }
        16 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::Reassign, vec![a], vec![v]);
            }
        }
        17 => {
            if let Some(a) = pick_var(rng, available) {
                let v = prog.new_var();
                prog.emit(Op::GetMetatable, vec![a], vec![v]);
            }
        }
        18 => {
            if let (Some(a), Some(b)) = (pick_var(rng, available), pick_var(rng, available)) {
                let v = prog.new_var();
                prog.emit(Op::SetMetatable, vec![a, b], vec![v]);
            }
        }
        _ => {
            prog.emit(Op::Nop, vec![], vec![]);
        }
    }
}

fn random_string(rng: &mut StdRng) -> String {
    let len = rng.random_range(0..20);
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789 \t\n\"\\"
        .chars()
        .collect();
    (0..len)
        .map(|_| chars[rng.random_range(0..chars.len())])
        .collect()
}

fn lift_single(op: Op, inputs: Vec<Variable>, outputs: Vec<Variable>) -> String {
    let mut prog = Program::new();
    for &v in &inputs {
        while prog.next_var <= v.0 {
            let var = prog.new_var();
            prog.emit(Op::LoadInt(0), vec![], vec![var]);
        }
    }
    for &v in &outputs {
        while prog.next_var <= v.0 {
            prog.new_var();
        }
    }
    prog.emit(op, inputs, outputs);
    lift(&prog)
}

fn last_line(lua: &str) -> &str {
    lua.lines().last().unwrap_or("")
}

// -- Fuzz harness --------------------------------------------------------

#[test]
fn fuzz_ir_roundtrip() {
    let mut rng = StdRng::seed_from_u64(FUZZ_SEED);

    for i in 0..FUZZ_ITERATIONS {
        let prog = random_program(&mut rng, 30);

        if let Err(errors) = prog.validate() {
            panic!("program {i} failed validation: {errors:?}\n{prog:?}");
        }

        let lua = lift(&prog);
        assert!(
            !lua.is_empty() || prog.instructions.iter().all(|i| matches!(i.op, Op::Nop)),
            "program {i} lifted to empty string but has non-Nop instructions"
        );

        let encoded = bincode::serialize(&prog).unwrap();
        let decoded: Program = bincode::deserialize(&encoded).unwrap();
        let lua2 = lift(&decoded);
        assert_eq!(
            lua, lua2,
            "program {i}: serde roundtrip produced different Lua"
        );

        for idx in 0..prog.instructions.len() {
            let scoped = prog.variables_in_scope_at(idx);
            for sv in &scoped {
                assert!(
                    sv.var.0 < prog.next_var,
                    "program {i}: scope var {} >= next_var {} at instruction {idx}",
                    sv.var,
                    prog.next_var
                );
            }
        }
    }
}

// -- Lifter: simple ops --------------------------------------------------

#[test]
fn lift_literals() {
    let cases: Vec<(Op, &str)> = vec![
        (Op::LoadNil, "local v0 = nil"),
        (Op::LoadBool(true), "local v0 = true"),
        (Op::LoadBool(false), "local v0 = false"),
        (Op::LoadInt(42), "local v0 = 42"),
        (Op::LoadInt(-7), "local v0 = -7"),
        (Op::LoadFloat(3.125), "local v0 = 3.125"),
        (Op::LoadFloat(f64::NAN), "local v0 = 0/0"),
        (Op::LoadFloat(f64::INFINITY), "local v0 = 1/0"),
        (Op::LoadFloat(f64::NEG_INFINITY), "local v0 = -1/0"),
        (Op::LoadString("hello".into()), r#"local v0 = "hello""#),
    ];
    for (op, expected) in cases {
        let lua = lift_single(op.clone(), vec![], vec![Variable(0)]);
        assert_eq!(last_line(&lua), expected, "op: {op:?}");
    }
}

#[test]
fn lift_string_escaping() {
    let cases: Vec<(&str, &str)> = vec![
        ("a\0b", r#"local v0 = "a\0b""#),
        ("say \"hi\"", r#"local v0 = "say \"hi\"""#),
        ("a\\b", r#"local v0 = "a\\b""#),
        ("line1\nline2", r#"local v0 = "line1\nline2""#),
    ];
    for (input, expected) in cases {
        let lua = lift_single(Op::LoadString(input.into()), vec![], vec![Variable(0)]);
        assert_eq!(last_line(&lua), expected, "input: {input:?}");
    }
}

#[test]
fn lift_table_ops() {
    let v0 = Variable(0);
    let v1 = Variable(1);
    let v2 = Variable(2);
    let v3 = Variable(3);

    let cases: Vec<(Op, Vec<Variable>, Vec<Variable>, &str)> = vec![
        (Op::CreateTable, vec![], vec![v0], "local v0 = {}"),
        (
            Op::TableSetField("x".into()),
            vec![v0, v1],
            vec![],
            r#"v0["x"] = v1"#,
        ),
        (
            Op::TableGetField("x".into()),
            vec![v0],
            vec![v1],
            r#"local v1 = v0["x"]"#,
        ),
        (Op::TableSetIndex, vec![v0, v1, v2], vec![], "v0[v1] = v2"),
        (
            Op::TableGetIndex,
            vec![v0, v1],
            vec![v2],
            "local v2 = v0[v1]",
        ),
        (
            Op::TableSetNumericField(3),
            vec![v0, v1],
            vec![],
            "v0[3] = v1",
        ),
        (
            Op::TableGetNumericField(3),
            vec![v0],
            vec![v1],
            "local v1 = v0[3]",
        ),
        (Op::Reassign, vec![v0], vec![v1], "local v1 = v0"),
        (
            Op::SetMetatable,
            vec![v0, v1],
            vec![v2],
            "local v2 = setmetatable(v0, v1)",
        ),
        (
            Op::GetMetatable,
            vec![v0],
            vec![v1],
            "local v1 = getmetatable(v0)",
        ),
        (
            Op::RawGet,
            vec![v0, v1],
            vec![v2],
            "local v2 = rawget(v0, v1)",
        ),
        (Op::RawSet, vec![v0, v1, v2], vec![], "rawset(v0, v1, v2)"),
        (
            Op::RawEqual,
            vec![v0, v1],
            vec![v2],
            "local v2 = rawequal(v0, v1)",
        ),
        (Op::Next, vec![v0], vec![v1], "local v1 = next(v0)"),
        (Op::Unpack, vec![v0], vec![v1], "local v1 = unpack(v0)"),
        (
            Op::Select,
            vec![v0, v1],
            vec![v2],
            "local v2 = select(v0, v1)",
        ),
        (Op::Ipairs, vec![v0], vec![v1], "local v1 = ipairs(v0)"),
        (Op::Pairs, vec![v0], vec![v1], "local v1 = pairs(v0)"),
        (Op::SetFenv, vec![v0, v1], vec![], "setfenv(v0, v1)"),
        (Op::GetFenv, vec![v0], vec![v1], "local v1 = getfenv(v0)"),
        (Op::TypeOf, vec![v0], vec![v1], "local v1 = type(v0)"),
        (Op::ToNumber, vec![v0], vec![v1], "local v1 = tonumber(v0)"),
        (Op::ToString, vec![v0], vec![v1], "local v1 = tostring(v0)"),
        (Op::Print, vec![v0], vec![], "print(v0)"),
        (
            Op::Loadstring,
            vec![v0],
            vec![v1],
            "local v1 = loadstring(v0)",
        ),
        (Op::Return, vec![], vec![], "return"),
        (Op::Return, vec![v0, v1], vec![], "return v0, v1"),
        (
            Op::CoroutineCreate,
            vec![v0],
            vec![v1],
            "local v1 = coroutine.create(v0)",
        ),
        (
            Op::CoroutineWrap,
            vec![v0],
            vec![v1],
            "local v1 = coroutine.wrap(v0)",
        ),
        (Op::CoroutineYield, vec![v0], vec![], "coroutine.yield(v0)"),
        (
            Op::CoroutineResume,
            vec![v0, v1],
            vec![v2, v3],
            "local v2, v3 = coroutine.resume(v0, v1)",
        ),
        (
            Op::StringLen,
            vec![v0],
            vec![v1],
            "local v1 = string.len(v0)",
        ),
        (
            Op::StringByte,
            vec![v0],
            vec![v1],
            "local v1 = string.byte(v0)",
        ),
        (
            Op::StringChar,
            vec![v0],
            vec![v1],
            "local v1 = string.char(v0)",
        ),
        (
            Op::StringRep,
            vec![v0, v1],
            vec![v2],
            "local v2 = string.rep(v0, v1)",
        ),
        (
            Op::StringSub,
            vec![v0, v1, v2],
            vec![v3],
            "local v3 = string.sub(v0, v1, v2)",
        ),
        (
            Op::StringFind,
            vec![v0, v1],
            vec![v2],
            "local v2 = string.find(v0, v1)",
        ),
        (
            Op::ToStringFmt("%d".into()),
            vec![v0],
            vec![v1],
            r#"local v1 = string.format("%d", v0)"#,
        ),
        (
            Op::StringFormat("%s %d".into()),
            vec![v0, v1],
            vec![v2],
            r#"local v2 = string.format("%s %d", v0, v1)"#,
        ),
        (
            Op::StringGmatch("%w+".into()),
            vec![v0],
            vec![v1],
            r#"local v1 = string.gmatch(v0, "%w+")"#,
        ),
        (
            Op::StringGsub("old".into(), "new".into()),
            vec![v0],
            vec![v1],
            r#"local v1 = string.gsub(v0, "old", "new")"#,
        ),
    ];
    for (op, inputs, outputs, expected) in cases {
        let lua = lift_single(op.clone(), inputs, outputs);
        assert_eq!(last_line(&lua), expected, "op: {op:?}");
    }
}

#[test]
fn lift_binary_ops() {
    for (op, sym) in [
        (BinOp::Add, "+"),
        (BinOp::Sub, "-"),
        (BinOp::Mul, "*"),
        (BinOp::Div, "/"),
        (BinOp::Mod, "%"),
        (BinOp::Pow, "^"),
        (BinOp::Concat, ".."),
    ] {
        let lua = lift_single(
            Op::BinaryOp(op),
            vec![Variable(0), Variable(1)],
            vec![Variable(2)],
        );
        let expected = format!("local v2 = v0 {sym} v1");
        assert_eq!(last_line(&lua), expected, "BinOp::{op:?}");
    }
}

#[test]
fn lift_unary_ops() {
    for (op, expected) in [
        (UnOp::Neg, "local v1 = -v0"),
        (UnOp::Not, "local v1 = not v0"),
        (UnOp::Len, "local v1 = #v0"),
    ] {
        let lua = lift_single(Op::UnaryOp(op), vec![Variable(0)], vec![Variable(1)]);
        assert_eq!(last_line(&lua), expected, "UnOp::{op:?}");
    }
}

#[test]
fn lift_compare_ops() {
    for (op, sym) in [
        (CmpOp::Eq, "=="),
        (CmpOp::Ne, "~="),
        (CmpOp::Lt, "<"),
        (CmpOp::Le, "<="),
        (CmpOp::Gt, ">"),
        (CmpOp::Ge, ">="),
    ] {
        let lua = lift_single(
            Op::Compare(op),
            vec![Variable(0), Variable(1)],
            vec![Variable(2)],
        );
        let expected = format!("local v2 = v0 {sym} v1");
        assert_eq!(last_line(&lua), expected, "CmpOp::{op:?}");
    }
}

#[test]
fn lift_gc_modes() {
    for (mode, expected) in [
        (GcMode::Collect, r#"collectgarbage("collect")"#),
        (GcMode::Stop, r#"collectgarbage("stop")"#),
        (GcMode::Restart, r#"collectgarbage("restart")"#),
        (GcMode::Step, r#"collectgarbage("step")"#),
    ] {
        let lua = lift_single(Op::CollectGarbage(mode), vec![], vec![]);
        assert_eq!(last_line(&lua), expected, "GcMode::{mode:?}");
    }
}

#[test]
fn lift_nop() {
    let mut prog = Program::new();
    prog.emit(Op::Nop, vec![], vec![]);
    assert_eq!(lift(&prog), "");
}

// -- Lifter: block structures --------------------------------------------

#[test]
fn lift_function() {
    let mut prog = Program::new();
    let f = prog.new_var();
    let p1 = prog.new_var();
    prog.emit(Op::BeginFunction { param_count: 1 }, vec![], vec![f, p1]);
    prog.emit(Op::Return, vec![p1], vec![]);
    prog.emit(Op::EndFunction, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("function(v1)"));
    assert!(lua.contains("return v1"));
    assert!(lua.contains("end"));
}

#[test]
fn lift_call_function() {
    let lua = lift_single(
        Op::CallFunction {
            arg_count: 1,
            ret_count: 1,
        },
        vec![Variable(0), Variable(1)],
        vec![Variable(2)],
    );
    assert_eq!(last_line(&lua), "local v2 = v0(v1)");

    let lua = lift_single(
        Op::CallFunction {
            arg_count: 0,
            ret_count: 0,
        },
        vec![Variable(0)],
        vec![],
    );
    assert_eq!(last_line(&lua), "v0()");
}

#[test]
fn lift_if_else() {
    let mut prog = Program::new();
    let cond = prog.new_var();
    prog.emit(Op::LoadBool(true), vec![], vec![cond]);
    prog.emit(Op::BeginIf, vec![cond], vec![]);
    let a = prog.new_var();
    prog.emit(Op::LoadInt(1), vec![], vec![a]);
    prog.emit(Op::BeginElse, vec![], vec![]);
    let b = prog.new_var();
    prog.emit(Op::LoadInt(2), vec![], vec![b]);
    prog.emit(Op::EndIf, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("if v0 then"));
    assert!(lua.contains("else"));
    assert!(lua.contains("end"));
}

#[test]
fn lift_while() {
    let mut prog = Program::new();
    let cond = prog.new_var();
    prog.emit(Op::LoadBool(true), vec![], vec![cond]);
    prog.emit(Op::BeginWhile, vec![cond], vec![]);
    prog.emit(Op::Break, vec![], vec![]);
    prog.emit(Op::EndWhile, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("while v0 do"));
    assert!(lua.contains("break"));
}

#[test]
fn lift_for_range() {
    let mut prog = Program::new();
    let start = prog.new_var();
    prog.emit(Op::LoadInt(1), vec![], vec![start]);
    let stop = prog.new_var();
    prog.emit(Op::LoadInt(10), vec![], vec![stop]);
    let step = prog.new_var();
    prog.emit(Op::LoadInt(2), vec![], vec![step]);
    let iter = prog.new_var();
    prog.emit(Op::BeginForRange, vec![start, stop, step], vec![iter]);
    prog.emit(Op::EndForRange, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("for v3 = v0, v1, v2 do"));
}

#[test]
fn lift_for_in() {
    let mut prog = Program::new();
    let t = prog.new_var();
    prog.emit(Op::CreateTable, vec![], vec![t]);
    let iter = prog.new_var();
    prog.emit(Op::Pairs, vec![t], vec![iter]);
    let k = prog.new_var();
    let v = prog.new_var();
    prog.emit(Op::BeginForIn, vec![iter], vec![k, v]);
    prog.emit(Op::EndForIn, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("for v2, v3 in v1 do"));
}

#[test]
fn lift_pcall() {
    let mut prog = Program::new();
    let result = prog.new_var();
    prog.emit(Op::BeginPcall, vec![], vec![result]);
    let x = prog.new_var();
    prog.emit(Op::LoadInt(42), vec![], vec![x]);
    prog.emit(Op::EndPcall, vec![], vec![]);

    let lua = lift(&prog);
    assert!(lua.contains("pcall(function()"));
    assert!(lua.contains("end)"));
}

// -- Scope ---------------------------------------------------------------

#[test]
fn scope_block_visibility() {
    let mut prog = Program::new();
    let outer = prog.new_var();
    prog.emit(Op::LoadInt(1), vec![], vec![outer]); // 0

    let cond = prog.new_var();
    prog.emit(Op::LoadBool(true), vec![], vec![cond]); // 1

    prog.emit(Op::BeginIf, vec![cond], vec![]); // 2
    let inner = prog.new_var();
    prog.emit(Op::LoadInt(42), vec![], vec![inner]); // 3
    prog.emit(Op::EndIf, vec![], vec![]); // 4

    prog.emit(Op::Nop, vec![], vec![]); // 5

    let scope_at_3 = prog.variables_in_scope_at(3);
    assert!(
        scope_at_3.iter().any(|sv| sv.var == outer),
        "outer visible inside block"
    );

    let scope_at_4 = prog.variables_in_scope_at(4);
    assert!(
        scope_at_4.iter().any(|sv| sv.var == inner),
        "inner visible at EndIf"
    );

    let scope_at_5 = prog.variables_in_scope_at(5);
    assert!(
        !scope_at_5.iter().any(|sv| sv.var == inner),
        "inner NOT visible after EndIf"
    );
    assert!(
        scope_at_5.iter().any(|sv| sv.var == outer),
        "outer still visible after EndIf"
    );
}

// -- Validation: rejection -----------------------------------------------

#[test]
fn validate_rejects_invalid_programs() {
    struct Case {
        name: &'static str,
        build: fn() -> Program,
        expect: &'static str,
    }

    let cases = [
        Case {
            name: "undefined variable",
            build: || {
                let mut p = Program::new();
                let v = p.new_var();
                p.emit(Op::Print, vec![v], vec![]);
                p
            },
            expect: "before definition",
        },
        Case {
            name: "unclosed if block",
            build: || {
                let mut p = Program::new();
                let v = p.new_var();
                p.emit(Op::LoadBool(true), vec![], vec![v]);
                p.emit(Op::BeginIf, vec![v], vec![]);
                p
            },
            expect: "unclosed",
        },
        Case {
            name: "end without begin",
            build: || {
                let mut p = Program::new();
                p.emit(Op::EndIf, vec![], vec![]);
                p
            },
            expect: "without matching",
        },
        Case {
            name: "mismatched block types",
            build: || {
                let mut p = Program::new();
                let v = p.new_var();
                p.emit(Op::LoadBool(true), vec![], vec![v]);
                p.emit(Op::BeginIf, vec![v], vec![]);
                p.emit(Op::EndWhile, vec![], vec![]);
                p
            },
            expect: "without matching",
        },
        Case {
            name: "bad next_var",
            build: || {
                let mut p = Program::new();
                let v = Variable(5);
                p.emit(Op::LoadInt(1), vec![], vec![v]);
                p.next_var = 3;
                p
            },
            expect: "next_var",
        },
        Case {
            name: "break outside loop",
            build: || {
                let mut p = Program::new();
                p.emit(Op::Break, vec![], vec![]);
                p
            },
            expect: "Break outside",
        },
        Case {
            name: "else without if",
            build: || {
                let mut p = Program::new();
                p.emit(Op::BeginElse, vec![], vec![]);
                p
            },
            expect: "BeginElse without",
        },
        Case {
            name: "arity error in deserialized program",
            build: || {
                let mut p = Program::new();
                let v = p.new_var();
                p.emit(Op::LoadInt(1), vec![], vec![v]);
                p.instructions.push(Instruction {
                    op: Op::BinaryOp(BinOp::Add),
                    inputs: vec![v],
                    outputs: vec![],
                });
                p
            },
            expect: "inputs",
        },
    ];

    for case in &cases {
        let prog = (case.build)();
        let errors = prog.validate().unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(case.expect)),
            "{}: expected error containing {:?}, got: {:?}",
            case.name,
            case.expect,
            errors
        );
    }
}

// -- Validation: acceptance ----------------------------------------------

#[test]
fn validate_accepts_valid_programs() {
    let mut break_in_loop = Program::new();
    let v = break_in_loop.new_var();
    break_in_loop.emit(Op::LoadBool(true), vec![], vec![v]);
    break_in_loop.emit(Op::BeginWhile, vec![v], vec![]);
    break_in_loop.emit(Op::Break, vec![], vec![]);
    break_in_loop.emit(Op::EndWhile, vec![], vec![]);

    let mut if_else = Program::new();
    let v = if_else.new_var();
    if_else.emit(Op::LoadBool(true), vec![], vec![v]);
    if_else.emit(Op::BeginIf, vec![v], vec![]);
    if_else.emit(Op::BeginElse, vec![], vec![]);
    if_else.emit(Op::EndIf, vec![], vec![]);

    for prog in [break_in_loop, if_else] {
        assert!(prog.validate().is_ok(), "program should be valid: {prog:?}");
    }
}

// -- Arity ---------------------------------------------------------------

#[test]
fn arity_validation() {
    assert!(
        Instruction::new(
            Op::BinaryOp(BinOp::Add),
            vec![Variable(0)],
            vec![Variable(1)]
        )
        .is_err(),
        "wrong input count"
    );
    assert!(
        Instruction::new(Op::LoadInt(1), vec![], vec![]).is_err(),
        "wrong output count"
    );
    assert!(
        Instruction::new(Op::Return, vec![Variable(0), Variable(1)], vec![]).is_ok(),
        "variable inputs"
    );
    assert!(
        Instruction::new(
            Op::CoroutineResume,
            vec![Variable(0)],
            vec![Variable(1), Variable(2), Variable(3)]
        )
        .is_ok(),
        "variable outputs"
    );
}

#[test]
#[should_panic(expected = "arity mismatch")]
fn emit_panics_on_arity_mismatch() {
    let mut prog = Program::new();
    prog.emit(Op::LoadInt(42), vec![], vec![]);
}

// -- Instruction display -------------------------------------------------

#[test]
fn instruction_display() {
    let with_outputs = Instruction::new(
        Op::BinaryOp(BinOp::Add),
        vec![Variable(0), Variable(1)],
        vec![Variable(2)],
    )
    .unwrap();
    assert_eq!(format!("{with_outputs}"), "v2 = BinaryOp(Add) v0, v1");

    let no_outputs = Instruction::new(Op::Print, vec![Variable(0)], vec![]).unwrap();
    assert_eq!(format!("{no_outputs}"), "Print v0");
}

// -- Block kind API ------------------------------------------------------

#[test]
fn block_ops_classification() {
    let block_pairs = [
        (Op::BeginIf, Op::EndIf),
        (Op::BeginWhile, Op::EndWhile),
        (Op::BeginForIn, Op::EndForIn),
        (Op::BeginForRange, Op::EndForRange),
        (Op::BeginFunction { param_count: 0 }, Op::EndFunction),
        (Op::BeginPcall, Op::EndPcall),
    ];
    for (begin, end) in &block_pairs {
        assert!(begin.opens_block().is_some(), "{begin:?} should open");
        assert!(end.closes_block().is_some(), "{end:?} should close");
    }
    assert!(Op::LoadInt(1).opens_block().is_none());
    assert!(Op::LoadInt(1).closes_block().is_none());
    assert!(Op::Nop.opens_block().is_none());
    assert!(Op::Nop.closes_block().is_none());
}
