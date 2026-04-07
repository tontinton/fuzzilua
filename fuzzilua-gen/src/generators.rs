use crate::builder::{ProgramBuilder, random_field_name, random_string};
use fuzzilua_ir::{BinOp, CmpOp, GcMode, Op, UnOp, Variable};
use rand::Rng;
use rand::RngCore;

type GenFn = fn(&mut ProgramBuilder, &mut dyn RngCore) -> Option<()>;

pub struct Generator {
    pub name: &'static str,
    pub weight: f64,
    pub generate: GenFn,
}

// -- Literals ----------------------------------------------------------------

fn gen_int(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::LoadInt(rng.random_range(-1000..1000)), vec![])?;
    Some(())
}

fn gen_float(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::LoadFloat(rng.random_range(-1000.0..1000.0)), vec![])?;
    Some(())
}

fn gen_string(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::LoadString(random_string(rng)), vec![])?;
    Some(())
}

fn gen_bool(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::LoadBool(rng.random_bool(0.5)), vec![])?;
    Some(())
}

fn gen_nil(b: &mut ProgramBuilder, _rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::LoadNil, vec![])?;
    Some(())
}

// -- Table -------------------------------------------------------------------

fn gen_create_table(b: &mut ProgramBuilder, _rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::CreateTable, vec![])?;
    Some(())
}

fn gen_get_property(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    b.emit(Op::TableGetField(random_field_name(rng)), vec![t])?;
    Some(())
}

fn gen_set_property(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let v = b.ensure_any(rng)?;
    b.emit(Op::TableSetField(random_field_name(rng)), vec![t, v])?;
    Some(())
}

fn gen_get_index(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let k = b.ensure_any(rng)?;
    b.emit(Op::TableGetIndex, vec![t, k])?;
    Some(())
}

fn gen_set_index(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let k = b.ensure_any(rng)?;
    let v = b.ensure_any(rng)?;
    b.emit(Op::TableSetIndex, vec![t, k, v])?;
    Some(())
}

// -- Metamethods -------------------------------------------------------------

const METAMETHODS: &[(&str, f64, u32)] = &[
    ("__index", 5.0, 2),
    ("__newindex", 5.0, 2),
    ("__eq", 4.0, 2),
    ("__concat", 4.0, 2),
    ("__len", 4.0, 1),
    ("__add", 5.0, 2),
    ("__call", 5.0, 1),
];

fn gen_metamethod_for(
    b: &mut ProgramBuilder,
    rng: &mut dyn RngCore,
    mm_name: &str,
    param_count: u32,
) -> Option<()> {
    let target = b.ensure_table(rng)?;
    let mt_var = b.emit(Op::CreateTable, vec![])?[0];

    if let Some(fn_outputs) = b.begin_block(Op::BeginFunction { param_count }, vec![]) {
        let fn_var = fn_outputs[0];
        if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            b.emit(Op::Return, vec![v]);
        }
        b.end_block();
        b.emit(Op::TableSetField(mm_name.to_string()), vec![mt_var, fn_var])?;
    }

    b.emit(Op::SetMetatable, vec![target, mt_var])?;
    Some(())
}

fn make_metamethod_gen(mm_name: &'static str, param_count: u32) -> GenFn {
    match (mm_name, param_count) {
        ("__index", 2) => |b, rng| gen_metamethod_for(b, rng, "__index", 2),
        ("__newindex", 2) => |b, rng| gen_metamethod_for(b, rng, "__newindex", 2),
        ("__eq", 2) => |b, rng| gen_metamethod_for(b, rng, "__eq", 2),
        ("__concat", 2) => |b, rng| gen_metamethod_for(b, rng, "__concat", 2),
        ("__len", 1) => |b, rng| gen_metamethod_for(b, rng, "__len", 1),
        ("__add", 2) => |b, rng| gen_metamethod_for(b, rng, "__add", 2),
        ("__call", 1) => |b, rng| gen_metamethod_for(b, rng, "__call", 1),
        _ => unreachable!(),
    }
}

// -- GC pressure -------------------------------------------------------------

fn gen_gc_collect(b: &mut ProgramBuilder, _rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::CollectGarbage(GcMode::Collect), vec![])?;
    Some(())
}

fn gen_gc_step(b: &mut ProgramBuilder, _rng: &mut dyn RngCore) -> Option<()> {
    b.emit(Op::CollectGarbage(GcMode::Step), vec![])?;
    Some(())
}

fn gen_alloc_pressure(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let count = rng.random_range(3..=8).min(b.remaining_budget());
    for _ in 0..count {
        if rng.random_bool(0.5) {
            b.emit(Op::CreateTable, vec![])?;
        } else {
            let s: String = (0..rng.random_range(10..=50))
                .map(|_| rng.random_range(b'a'..=b'z') as char)
                .collect();
            b.emit(Op::LoadString(s), vec![])?;
        }
    }
    Some(())
}

// -- Functions ---------------------------------------------------------------

fn gen_begin_function(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let param_count = rng.random_range(0..=3) as u32;
    b.begin_block(Op::BeginFunction { param_count }, vec![])?;
    let body_len = rng
        .random_range(1..=3)
        .min(b.remaining_budget().saturating_sub(1));
    for _ in 0..body_len {
        if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            let ops = [
                Op::LoadInt(rng.random_range(-10..10)),
                Op::BinaryOp(BinOp::Add),
                Op::UnaryOp(UnOp::Len),
            ];
            let idx = rng.random_range(0..ops.len());
            match &ops[idx] {
                Op::LoadInt(_) => {
                    b.emit(ops[idx].clone(), vec![]);
                }
                Op::BinaryOp(_) => {
                    if let Some(v2) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng)
                    {
                        b.emit(ops[idx].clone(), vec![v, v2]);
                    }
                }
                _ => {
                    b.emit(ops[idx].clone(), vec![v]);
                }
            }
        } else {
            b.emit(Op::LoadInt(rng.random_range(0..100)), vec![]);
        }
    }
    if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
        b.emit(Op::Return, vec![v]);
    }
    b.end_block();
    Some(())
}

fn gen_call_function(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let f = b.ensure_function(rng)?;
    let arg_count = rng.random_range(0..=2) as u32;
    let mut inputs = vec![f];
    for _ in 0..arg_count {
        if let Some(v) = b.ensure_any(rng) {
            inputs.push(v);
        }
    }
    let actual_args = (inputs.len() - 1) as u32;
    let ret_count = rng.random_range(0..=2) as u32;
    b.emit_n(
        Op::CallFunction {
            arg_count: actual_args,
            ret_count,
        },
        inputs,
        ret_count as usize,
    )?;
    Some(())
}

// -- Control flow ------------------------------------------------------------

fn gen_if(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let cond = b.ensure_any(rng)?;
    b.begin_block(Op::BeginIf, vec![cond])?;
    b.emit(Op::LoadInt(rng.random_range(0..100)), vec![]);
    if rng.random_bool(0.3) {
        b.begin_else();
        b.emit(Op::LoadInt(rng.random_range(0..100)), vec![]);
    }
    b.end_block();
    Some(())
}

fn gen_while(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let cond_var = b.emit(Op::LoadBool(false), vec![])?[0];
    b.begin_block(Op::BeginWhile, vec![cond_var])?;
    b.emit(Op::LoadInt(rng.random_range(0..10)), vec![]);
    b.emit(Op::Break, vec![]);
    b.end_block();
    Some(())
}

fn gen_for_numeric(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let start = b.ensure_number(rng)?;
    let limit_var = b.emit(Op::LoadInt(rng.random_range(1..5)), vec![])?[0];
    b.begin_block(Op::BeginForRange, vec![start, limit_var])?;
    b.emit(Op::LoadInt(0), vec![]);
    b.end_block();
    Some(())
}

fn gen_for_generic(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let iter_var = b.emit(Op::Pairs, vec![t])?[0];
    b.begin_block(Op::BeginForIn, vec![iter_var])?;
    b.emit(Op::LoadInt(rng.random_range(0..10)), vec![]);
    b.end_block();
    Some(())
}

// -- Callbacks with GC -------------------------------------------------------

fn emit_loadstring_call(
    b: &mut ProgramBuilder,
    lua: &str,
    args: Vec<Variable>,
    ret_count: u32,
) -> Option<Vec<Variable>> {
    let code_var = b.emit(Op::LoadString(lua.to_string()), vec![])?[0];
    let loader_var = b.emit(Op::Loadstring, vec![code_var])?[0];
    let factory = b.emit_n(
        Op::CallFunction {
            arg_count: 0,
            ret_count: 1,
        },
        vec![loader_var],
        1,
    )?;
    let f = factory[0];
    let arg_count = args.len() as u32;
    let mut inputs = vec![f];
    inputs.extend(args);
    b.emit_n(
        Op::CallFunction {
            arg_count,
            ret_count,
        },
        inputs,
        ret_count as usize,
    )
}

fn gen_sort_comparator(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let count = rng.random_range(3..=6).min(b.remaining_budget());
    for i in 1..=count {
        let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
        b.emit(Op::TableSetNumericField(i as i64), vec![t, v])?;
    }
    let lua = "return function(t) table.sort(t, function(a,b) collectgarbage('step') return a < b end) end";
    emit_loadstring_call(b, lua, vec![t], 0)?;
    Some(())
}

fn gen_gsub_callback(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let s = b.ensure_string(rng)?;
    let lua = "return function(s) return string.gsub(s, '.', function(c) collectgarbage('step') return c end) end";
    emit_loadstring_call(b, lua, vec![s], 1)?;
    Some(())
}

fn gen_metamethod_callback_with_gc(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let target = b.ensure_table(rng)?;
    let mt_var = b.emit(Op::CreateTable, vec![])?[0];

    let mm_names = [
        "__index",
        "__newindex",
        "__add",
        "__eq",
        "__len",
        "__concat",
        "__call",
    ];
    let mm = mm_names[rng.random_range(0..mm_names.len())];

    let param_count = if mm == "__len" || mm == "__call" {
        1u32
    } else {
        2
    };

    if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count }, vec![]) {
        let fn_var = fn_out[0];
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
        for _ in 0..rng.random_range(1..=3) {
            b.emit(Op::CreateTable, vec![]);
        }
        if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            b.emit(Op::Return, vec![v]);
        }
        b.end_block();
        b.emit(Op::TableSetField(mm.to_string()), vec![mt_var, fn_var]);
    }

    b.emit(Op::SetMetatable, vec![target, mt_var])?;
    trigger_metamethod(b, rng, target, mm);
    Some(())
}

fn trigger_metamethod(b: &mut ProgramBuilder, rng: &mut dyn RngCore, target: Variable, mm: &str) {
    match mm {
        "__index" => {
            b.emit(Op::TableGetField(random_field_name(rng)), vec![target]);
        }
        "__newindex" => {
            if let Some(v) = b.ensure_any(rng) {
                b.emit(Op::TableSetField(random_field_name(rng)), vec![target, v]);
            }
        }
        "__add" => {
            if let Some(v) = b.ensure_any(rng) {
                b.emit(Op::BinaryOp(BinOp::Add), vec![target, v]);
            }
        }
        "__eq" => {
            if let Some(v) = b.ensure_any(rng) {
                b.emit(Op::Compare(CmpOp::Eq), vec![target, v]);
            }
        }
        "__len" => {
            b.emit(Op::UnaryOp(UnOp::Len), vec![target]);
        }
        "__concat" => {
            if let Some(v) = b.ensure_any(rng) {
                b.emit(Op::BinaryOp(BinOp::Concat), vec![target, v]);
            }
        }
        "__call" => {
            b.emit_n(
                Op::CallFunction {
                    arg_count: 0,
                    ret_count: 1,
                },
                vec![target],
                1,
            );
        }
        _ => {}
    }
}

// -- Coroutine ---------------------------------------------------------------

fn gen_coroutine_basic(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let f = b.ensure_function(rng)?;
    let co_var = b.emit(Op::CoroutineCreate, vec![f])?[0];
    b.emit_n(Op::CoroutineResume, vec![co_var], rng.random_range(1..=2))?;
    Some(())
}

// -- Loadstring --------------------------------------------------------------

fn gen_loadstring_simple(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let snippets = [
        "return 1 + 2",
        "local t = {}; t.x = 1; return t",
        "collectgarbage('step'); return true",
        "local s = string.rep('x', 100); return #s",
    ];
    let code = snippets[rng.random_range(0..snippets.len())];
    let s_var = b.emit(Op::LoadString(code.to_string()), vec![])?[0];
    let f_var = b.emit(Op::Loadstring, vec![s_var])?[0];
    b.emit_n(
        Op::CallFunction {
            arg_count: 0,
            ret_count: 1,
        },
        vec![f_var],
        1,
    )?;
    Some(())
}

// -- Upvalue -----------------------------------------------------------------

fn gen_upvalue(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let outer_var = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
    let fn_out = b.begin_block(Op::BeginFunction { param_count: 0 }, vec![])?;
    let fn_var = fn_out[0];
    b.emit(Op::Reassign, vec![outer_var]);
    if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
        b.emit(Op::Return, vec![v]);
    }
    b.end_block();
    b.emit_n(
        Op::CallFunction {
            arg_count: 0,
            ret_count: 1,
        },
        vec![fn_var],
        1,
    )?;
    Some(())
}

// -- Registry ----------------------------------------------------------------

pub fn all_generators() -> Vec<Generator> {
    let mut gens = vec![
        Generator {
            name: "int",
            weight: 1.0,
            generate: gen_int,
        },
        Generator {
            name: "float",
            weight: 1.0,
            generate: gen_float,
        },
        Generator {
            name: "string",
            weight: 1.0,
            generate: gen_string,
        },
        Generator {
            name: "bool",
            weight: 1.0,
            generate: gen_bool,
        },
        Generator {
            name: "nil",
            weight: 1.0,
            generate: gen_nil,
        },
        Generator {
            name: "create_table",
            weight: 3.0,
            generate: gen_create_table,
        },
        Generator {
            name: "get_property",
            weight: 3.0,
            generate: gen_get_property,
        },
        Generator {
            name: "set_property",
            weight: 3.0,
            generate: gen_set_property,
        },
        Generator {
            name: "get_index",
            weight: 2.0,
            generate: gen_get_index,
        },
        Generator {
            name: "set_index",
            weight: 2.0,
            generate: gen_set_index,
        },
        Generator {
            name: "gc_collect",
            weight: 5.0,
            generate: gen_gc_collect,
        },
        Generator {
            name: "gc_step",
            weight: 5.0,
            generate: gen_gc_step,
        },
        Generator {
            name: "alloc_pressure",
            weight: 5.0,
            generate: gen_alloc_pressure,
        },
        Generator {
            name: "begin_function",
            weight: 3.0,
            generate: gen_begin_function,
        },
        Generator {
            name: "call_function",
            weight: 3.0,
            generate: gen_call_function,
        },
        Generator {
            name: "if",
            weight: 1.0,
            generate: gen_if,
        },
        Generator {
            name: "while",
            weight: 1.0,
            generate: gen_while,
        },
        Generator {
            name: "for_numeric",
            weight: 1.0,
            generate: gen_for_numeric,
        },
        Generator {
            name: "for_generic",
            weight: 1.0,
            generate: gen_for_generic,
        },
        Generator {
            name: "sort_comparator",
            weight: 5.0,
            generate: gen_sort_comparator,
        },
        Generator {
            name: "gsub_callback",
            weight: 5.0,
            generate: gen_gsub_callback,
        },
        Generator {
            name: "metamethod_gc",
            weight: 6.0,
            generate: gen_metamethod_callback_with_gc,
        },
        Generator {
            name: "coroutine",
            weight: 3.0,
            generate: gen_coroutine_basic,
        },
        Generator {
            name: "loadstring",
            weight: 3.0,
            generate: gen_loadstring_simple,
        },
        Generator {
            name: "upvalue",
            weight: 3.0,
            generate: gen_upvalue,
        },
    ];

    for &(mm_name, weight, param_count) in METAMETHODS {
        gens.push(Generator {
            name: mm_name,
            weight,
            generate: make_metamethod_gen(mm_name, param_count),
        });
    }

    gens
}
