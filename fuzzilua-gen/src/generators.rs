use crate::builder::{ProgramBuilder, random_field_name, random_string};
use fuzzilua_ir::{BinOp, CmpOp, GcMode, METAMETHODS, Op, UnOp, Variable};
use rand::Rng;
use rand::RngCore;
use std::sync::Arc;

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
    b.emit(Op::LoadString(random_string(rng).into()), vec![])?;
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
    b.emit(Op::TableGetField(random_field_name(rng).into()), vec![t])?;
    Some(())
}

fn gen_set_property(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let v = b.ensure_any(rng)?;
    b.emit(Op::TableSetField(random_field_name(rng).into()), vec![t, v])?;
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

const GENERATOR_METAMETHODS: &[(&str, f64)] = &[
    ("__index", 5.0),
    ("__newindex", 5.0),
    ("__eq", 4.0),
    ("__concat", 4.0),
    ("__len", 4.0),
    ("__add", 5.0),
    ("__call", 5.0),
];

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
            b.emit(Op::LoadString(s.into()), vec![])?;
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

pub(crate) fn emit_loadstring_call(
    b: &mut ProgramBuilder,
    lua: &str,
    args: Vec<Variable>,
    ret_count: u32,
) -> Option<Vec<Variable>> {
    let code_var = b.emit(Op::LoadString(lua.into()), vec![])?[0];
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

    let mm = GENERATOR_METAMETHODS[rng.random_range(0..GENERATOR_METAMETHODS.len())].0;
    let param_count = METAMETHODS
        .iter()
        .find(|m| m.name == mm)
        .map_or(2, |m| m.param_count);

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
        b.emit(Op::TableSetField(mm.into()), vec![mt_var, fn_var]);
    }

    b.emit(Op::SetMetatable, vec![target, mt_var])?;
    trigger_metamethod(b, rng, target, mm);
    Some(())
}

pub(crate) fn trigger_metamethod(
    b: &mut ProgramBuilder,
    rng: &mut dyn RngCore,
    target: Variable,
    mm: &str,
) {
    match mm {
        "__index" => {
            b.emit(
                Op::TableGetField(random_field_name(rng).into()),
                vec![target],
            );
        }
        "__newindex" => {
            if let Some(v) = b.ensure_any(rng) {
                b.emit(
                    Op::TableSetField(random_field_name(rng).into()),
                    vec![target, v],
                );
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

// -- Error / pcall -----------------------------------------------------------

fn gen_error_throw(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let msg = b.emit(Op::LoadString(Arc::from(random_string(rng))), vec![])?[0];
    let cond = b.emit(Op::LoadBool(rng.random_bool(0.5)), vec![])?[0];
    b.begin_block(Op::BeginIf, vec![cond])?;
    b.emit(Op::Error, vec![msg]);
    b.end_block();
    Some(())
}

fn gen_pcall_wrap(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let _ok_var = b.begin_block(Op::BeginPcall, vec![])?[0];
    let budget = rng
        .random_range(1..=3)
        .min(b.remaining_budget().saturating_sub(1));
    for _ in 0..budget {
        if rng.random_bool(0.4) {
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
        } else if rng.random_bool(0.3) {
            if let Some(t) = b.random_variable_of_type(fuzzilua_ir::LuaType::Table, rng) {
                b.emit(Op::TableGetField(random_field_name(rng).into()), vec![t]);
            }
        } else {
            let msg = b.emit(Op::LoadString(Arc::from("err")), vec![])?[0];
            b.emit(Op::Error, vec![msg]);
        }
    }
    b.end_block();
    Some(())
}

// -- Property removal (t[k] = nil) -------------------------------------------

fn gen_property_remove(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let nil_var = b.emit(Op::LoadNil, vec![])?[0];
    if rng.random_bool(0.5) {
        b.emit(
            Op::TableSetField(random_field_name(rng).into()),
            vec![t, nil_var],
        )?;
    } else {
        let idx = b.emit(Op::LoadInt(rng.random_range(1..=10)), vec![])?[0];
        b.emit(Op::TableSetIndex, vec![t, idx, nil_var])?;
    }
    Some(())
}

// -- Reassign (alias creation) -----------------------------------------------

fn gen_reassign(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let src = b.ensure_any(rng)?;
    b.emit(Op::Reassign, vec![src])?;
    Some(())
}

// -- Metatable swap ----------------------------------------------------------

fn gen_metatable_swap(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let new_mt = b.emit(Op::CreateTable, vec![])?[0];
    let mm = GENERATOR_METAMETHODS[rng.random_range(0..GENERATOR_METAMETHODS.len())].0;
    let param_count = METAMETHODS
        .iter()
        .find(|m| m.name == mm)
        .map_or(2, |m| m.param_count);
    if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count }, vec![]) {
        let fn_var = fn_out[0];
        if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            b.emit(Op::Return, vec![v]);
        }
        b.end_block();
        b.emit(Op::TableSetField(mm.into()), vec![new_mt, fn_var]);
    }
    b.emit(Op::SetMetatable, vec![t, new_mt])?;
    trigger_metamethod(b, rng, t, mm);
    let replace_mt = b.emit(Op::CreateTable, vec![])?[0];
    b.emit(Op::SetMetatable, vec![t, replace_mt])?;
    trigger_metamethod(b, rng, t, mm);
    Some(())
}

// -- Table resize (insert/remove) --------------------------------------------

fn gen_table_resize(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let count = rng.random_range(2..=5).min(b.remaining_budget());
    for i in 0..count {
        if rng.random_bool(0.6) {
            let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
            b.emit(Op::TableInsert, vec![t, v])?;
        } else {
            b.emit(Op::TableRemove, vec![t])?;
        }
        if i > 0 && rng.random_bool(0.3) {
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
        }
    }
    Some(())
}

// -- Weak table (__mode) -----------------------------------------------------

fn gen_weak_table(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.emit(Op::CreateTable, vec![])?[0];
    let mt = b.emit(Op::CreateTable, vec![])?[0];
    let modes = ["k", "v", "kv"];
    let mode = modes[rng.random_range(0..modes.len())];
    let mode_str = b.emit(Op::LoadString(Arc::from(mode)), vec![])?[0];
    b.emit(Op::TableSetField("__mode".into()), vec![mt, mode_str])?;
    b.emit(Op::SetMetatable, vec![t, mt])?;
    let count = rng.random_range(2..=4).min(b.remaining_budget());
    for _ in 0..count {
        let key = b.emit(Op::CreateTable, vec![])?[0];
        let val = b.emit(Op::CreateTable, vec![])?[0];
        b.emit(Op::TableSetIndex, vec![t, key, val])?;
    }
    b.emit(Op::CollectGarbage(GcMode::Collect), vec![])?;
    if let Some(k) = b.random_variable_of_type(fuzzilua_ir::LuaType::Table, rng) {
        b.emit(Op::TableGetIndex, vec![t, k]);
    }
    Some(())
}

// -- Finalizer (__gc via newproxy) -------------------------------------------

// -- Loadstring --------------------------------------------------------------

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

// -- Stdlib overwrite --------------------------------------------------------

fn gen_stdlib_overwrite(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let overwrites = [
        "table.sort = function() end",
        "table.insert = function() end",
        "table.remove = function() return nil end",
        "string.gsub = function(s) return s, 0 end",
        "string.sub = function(s) return s end",
        "string.find = function() return nil end",
        "string.format = tostring",
        "tostring = function(x) return '' end",
        "tonumber = function(x) return 0 end",
        "rawget = function(t,k) return t[k] end",
        "type = function() return 'table' end",
        "setmetatable = function(t) return t end",
        "next = function() return nil end",
        "select = function() return nil end",
        "unpack = function() return nil end",
        "pcall = function(f, ...) return true, f(...) end",
    ];
    let snippet = overwrites[rng.random_range(0..overwrites.len())];
    let lua = format!("return function() {snippet} end");
    emit_loadstring_call(b, &lua, vec![], 0)?;
    Some(())
}

// -- Coroutine + GC interleave -----------------------------------------------

fn gen_coroutine_gc_interleave(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let target = b.ensure_table(rng)?;
    let mt = b.emit(Op::CreateTable, vec![])?[0];

    let mm_choices = ["__index", "__newindex", "__add", "__len", "__concat"];
    let mm = mm_choices[rng.random_range(0..mm_choices.len())];
    let param_count = METAMETHODS
        .iter()
        .find(|m| m.name == mm)
        .map_or(2, |m| m.param_count);

    let fn_out = b.begin_block(Op::BeginFunction { param_count }, vec![])?;
    let mm_fn = fn_out[0];
    b.emit(Op::CoroutineYield, vec![]);
    b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
    for _ in 0..rng.random_range(1..=3) {
        b.emit(Op::CreateTable, vec![]);
    }
    if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
        b.emit(Op::Return, vec![v]);
    }
    b.end_block();

    b.emit(Op::TableSetField(mm.into()), vec![mt, mm_fn])?;
    b.emit(Op::SetMetatable, vec![target, mt])?;

    let wrapper_out = b.begin_block(Op::BeginFunction { param_count: 0 }, vec![])?;
    let wrapper_fn = wrapper_out[0];
    trigger_metamethod(b, rng, target, mm);
    b.end_block();

    let co = b.emit(Op::CoroutineCreate, vec![wrapper_fn])?[0];
    b.emit_n(Op::CoroutineResume, vec![co], 1)?;
    b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    b.emit_n(Op::CoroutineResume, vec![co], 1)?;
    Some(())
}

// -- Method call via table ---------------------------------------------------

fn gen_method_call_via_table(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t1 = b.ensure_table(rng)?;
    let fn_out = b.begin_block(Op::BeginFunction { param_count: 1 }, vec![])?;
    let method_fn = fn_out[0];
    let self_param = fn_out[1];
    b.emit(Op::TableGetField("x".into()), vec![self_param]);
    if rng.random_bool(0.5) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }
    if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
        b.emit(Op::Return, vec![v]);
    }
    b.end_block();

    let field = random_field_name(rng);
    b.emit(Op::TableSetField(field.clone().into()), vec![t1, method_fn])?;

    let extracted = b.emit(Op::TableGetField(field.into()), vec![t1])?[0];

    let t2 = b.emit(Op::CreateTable, vec![])?[0];
    let val = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
    b.emit(Op::TableSetField("x".into()), vec![t2, val])?;

    if rng.random_bool(0.5) {
        let mt = b.emit(Op::CreateTable, vec![])?[0];
        let mm = GENERATOR_METAMETHODS[rng.random_range(0..GENERATOR_METAMETHODS.len())].0;
        let mm_param_count = METAMETHODS
            .iter()
            .find(|m| m.name == mm)
            .map_or(2, |m| m.param_count);
        if let Some(mm_out) = b.begin_block(
            Op::BeginFunction {
                param_count: mm_param_count,
            },
            vec![],
        ) {
            let mm_fn = mm_out[0];
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
            if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
                b.emit(Op::Return, vec![v]);
            }
            b.end_block();
            b.emit(Op::TableSetField(mm.into()), vec![mt, mm_fn]);
        }
        b.emit(Op::SetMetatable, vec![t2, mt]);
    }

    b.emit_n(
        Op::CallFunction {
            arg_count: 1,
            ret_count: 1,
        },
        vec![extracted, t2],
        1,
    )?;

    if rng.random_bool(0.3) {
        let via_index_lua = "return function(t1, t2) \
            local mt = { __index = t1 } \
            setmetatable(t2, mt) \
            return t2 \
        end";
        let t3 = b.emit(Op::CreateTable, vec![])?[0];
        emit_loadstring_call(b, via_index_lua, vec![t1, t3], 1)?;
    }
    Some(())
}

// -- Number computation chain ------------------------------------------------

// -- Compare with branch -----------------------------------------------------

fn gen_compare_with_branch(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let a = if rng.random_bool(0.5) {
        b.ensure_table(rng)?
    } else {
        b.ensure_any(rng)?
    };
    let bb = if rng.random_bool(0.5) {
        b.ensure_table(rng)?
    } else {
        b.ensure_any(rng)?
    };

    let cmp_ops = [CmpOp::Eq, CmpOp::Lt, CmpOp::Le];
    let cmp = cmp_ops[rng.random_range(0..cmp_ops.len())];
    let cond = b.emit(Op::Compare(cmp), vec![a, bb])?[0];

    b.begin_block(Op::BeginIf, vec![cond])?;
    if rng.random_bool(0.5) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }
    let cmp2 = cmp_ops[rng.random_range(0..cmp_ops.len())];
    b.emit(Op::Compare(cmp2), vec![a, bb]);
    if rng.random_bool(0.5) {
        b.begin_else();
        let cmp3 = cmp_ops[rng.random_range(0..cmp_ops.len())];
        b.emit(Op::Compare(cmp3), vec![bb, a]);
    }
    b.end_block();
    Some(())
}

// -- Custom iterator ---------------------------------------------------------

fn gen_custom_iterator(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let count = rng.random_range(2..=5).min(b.remaining_budget());
    for i in 1..=count {
        let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
        b.emit(Op::TableSetNumericField(i as i64), vec![t, v])?;
    }
    let lua = "return function(t) \
        local i = 0 \
        return function() \
            i = i + 1 \
            local v = t[i] \
            if v then return i, v end \
        end \
    end";
    let iter = emit_loadstring_call(b, lua, vec![t], 1)?;
    let iter_fn = iter[0];
    b.begin_block(Op::BeginForIn, vec![iter_fn])?;
    if rng.random_bool(0.4) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }
    if rng.random_bool(0.3) {
        let v = b.emit(Op::LoadInt(rng.random_range(0..50)), vec![])?[0];
        b.emit(Op::TableInsert, vec![t, v]);
    }
    b.end_block();
    Some(())
}

// -- Metatable hierarchy -----------------------------------------------------

fn gen_metatable_hierarchy(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let depth = rng.random_range(3..=4).min(b.remaining_budget() / 3);
    if depth < 2 {
        return None;
    }

    let mut tables: Vec<Variable> = Vec::with_capacity(depth);
    for _ in 0..depth {
        tables.push(b.emit(Op::CreateTable, vec![])?[0]);
    }

    for i in (1..tables.len()).rev() {
        let child_mt = b.emit(Op::CreateTable, vec![])?[0];
        b.emit(
            Op::TableSetField("__index".into()),
            vec![child_mt, tables[i - 1]],
        )?;
        b.emit(Op::SetMetatable, vec![tables[i], child_mt])?;
    }

    let val = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
    let field: Arc<str> = random_field_name(rng).into();
    b.emit(Op::TableSetField(field.clone()), vec![tables[0], val])?;

    let _result = b.emit(Op::TableGetField(field), vec![*tables.last().unwrap()])?;

    if rng.random_bool(0.5) {
        let mid = rng.random_range(0..tables.len().saturating_sub(1).max(1));
        let new_mt = b.emit(Op::CreateTable, vec![])?[0];
        b.emit(Op::SetMetatable, vec![tables[mid], new_mt])?;
        b.emit(
            Op::TableGetField(random_field_name(rng).into()),
            vec![*tables.last().unwrap()],
        );
    }

    if rng.random_bool(0.3) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }
    Some(())
}

// -- Table populate (array-to-hash migration) --------------------------------

fn gen_table_populate(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.emit(Op::CreateTable, vec![])?[0];
    let count = rng
        .random_range(5..=20)
        .min(b.remaining_budget().saturating_sub(3));
    if count < 3 {
        return None;
    }

    for i in 1..=count {
        let v = b.emit(Op::LoadInt(rng.random_range(0..1000)), vec![])?[0];
        b.emit(Op::TableSetNumericField(i as i64), vec![t, v])?;
    }

    let hash_val = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
    b.emit(
        Op::TableSetField(random_field_name(rng).into()),
        vec![t, hash_val],
    )?;

    if rng.random_bool(0.4) {
        let nil = b.emit(Op::LoadNil, vec![])?[0];
        let gap = rng.random_range(1..=count) as i64;
        b.emit(Op::TableSetNumericField(gap), vec![t, nil])?;
    }

    if rng.random_bool(0.3) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }

    b.emit(Op::UnaryOp(UnOp::Len), vec![t])?;
    Some(())
}

// -- Mutation during iteration -----------------------------------------------

fn gen_mutation_during_iteration(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let t = b.ensure_table(rng)?;
    let fill = rng
        .random_range(3..=8)
        .min(b.remaining_budget().saturating_sub(6));
    if fill < 2 {
        return None;
    }
    for i in 1..=fill {
        let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
        b.emit(Op::TableSetNumericField(i as i64), vec![t, v])?;
    }

    let iter = b.emit(Op::Pairs, vec![t])?[0];
    b.begin_block(Op::BeginForIn, vec![iter])?;

    match rng.random_range(0..3) {
        0 => {
            let nil = b.emit(Op::LoadNil, vec![])?[0];
            let key = b.emit(Op::LoadInt(rng.random_range(1..=fill as i64)), vec![])?[0];
            b.emit(Op::TableSetIndex, vec![t, key, nil]);
        }
        1 => {
            let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
            b.emit(Op::TableSetField(random_field_name(rng).into()), vec![t, v]);
        }
        _ => {
            let v = b.emit(Op::LoadInt(rng.random_range(0..100)), vec![])?[0];
            b.emit(Op::TableInsert, vec![t, v]);
        }
    }

    if rng.random_bool(0.3) {
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    }
    b.end_block();
    Some(())
}

// -- Xpcall handler ----------------------------------------------------------

fn gen_xpcall_handler(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let target = b.ensure_table(rng)?;
    let handler_variants = [
        "return function(t) \
            xpcall(function() \
                error(tostring(t)) \
            end, function(err) \
                error('double fault: ' .. tostring(err)) \
            end) \
        end",
        "return function(t) \
            xpcall(function() \
                t[nil] = t \
                error('fail') \
            end, function(err) \
                collectgarbage('collect') \
                collectgarbage('collect') \
                return err \
            end) \
        end",
        "return function(t) \
            xpcall(function() \
                error(t) \
            end, function(err) \
                for i=1,10 do t[i] = {} end \
                collectgarbage('step') \
                return err \
            end) \
        end",
    ];
    let lua = handler_variants[rng.random_range(0..handler_variants.len())];
    emit_loadstring_call(b, lua, vec![target], 0)?;
    if rng.random_bool(0.3) {
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
    }
    Some(())
}

// -- string.format + __tostring GC -------------------------------------------

fn gen_string_format_tostring(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let target = b.ensure_table(rng)?;
    let mt = b.emit(Op::CreateTable, vec![])?[0];

    if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 1 }, vec![]) {
        let fn_var = fn_out[0];
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
        for _ in 0..rng.random_range(1..=3) {
            b.emit(Op::CreateTable, vec![]);
        }
        let s = b.emit(Op::LoadString("fmt_result".into()), vec![])?[0];
        b.emit(Op::Return, vec![s]);
        b.end_block();
        b.emit(Op::TableSetField("__tostring".into()), vec![mt, fn_var]);
    }
    b.emit(Op::SetMetatable, vec![target, mt])?;

    let fmts = ["%s", "%s %s", "val=%s key=%s"];
    let fmt: Arc<str> = fmts[rng.random_range(0..fmts.len())].into();
    let arg_count = fmt.matches("%s").count() as u32;
    let mut args = vec![target];
    for _ in 1..arg_count {
        if rng.random_bool(0.5) {
            args.push(target);
        } else if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            args.push(v);
        } else {
            args.push(target);
        }
    }
    b.emit_n(Op::StringFormat(fmt), args, 1)?;
    Some(())
}

// -- table.concat + __tostring GC --------------------------------------------

fn gen_table_concat_tostring(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let count = rng.random_range(4..=10);
    let lua = format!(
        "return function() \
            local mt = {{ __tostring = function() collectgarbage('step'); \
                for i=1,5 do local _={{}} end; return 'x' end }} \
            local t = {{}} \
            for i=1,{count} do \
                local o = setmetatable({{}}, mt) \
                t[i] = o \
            end \
            return table.concat(t, ',') \
        end"
    );
    emit_loadstring_call(b, &lua, vec![], 1)?;
    b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
    Some(())
}

// -- Deep __index chain via coroutine ----------------------------------------

fn gen_deep_index_coroutine(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let depth = rng.random_range(50..=100);
    let lua = format!(
        "return function() \
            local function build(n) \
                local t = {{}} \
                local cur = t \
                for i=1,{depth} do \
                    local inner = {{}} \
                    setmetatable(cur, {{ __index = function(self, k) \
                        collectgarbage('step') \
                        return inner[k] \
                    end }}) \
                    cur = inner \
                end \
                cur.val = 42 \
                return t \
            end \
            local co = coroutine.create(function() \
                local chain = build({depth}) \
                coroutine.yield() \
                collectgarbage('collect') \
                local _ = chain.val \
            end) \
            coroutine.resume(co) \
            collectgarbage('step') \
            coroutine.resume(co) \
        end"
    );
    emit_loadstring_call(b, &lua, vec![], 0)?;
    Some(())
}

// -- error() with __tostring GC ----------------------------------------------

fn gen_error_tostring_gc(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let variants = [
        "return function() \
            local mt = { __tostring = function() collectgarbage('collect'); \
                for i=1,8 do local _={} end; return 'err' end } \
            local e = setmetatable({}, mt) \
            error(e) \
        end",
        "return function() \
            local mt = { __tostring = function(self) \
                collectgarbage('step') \
                self.n = (self.n or 0) + 1 \
                return string.rep('x', self.n * 100) \
            end } \
            local e = setmetatable({n=0}, mt) \
            local ok, msg = pcall(error, e) \
            local s = tostring(msg) \
            collectgarbage('collect') \
            local _ = s \
        end",
        "return function() \
            local mt = { __tostring = function() \
                collectgarbage('collect') \
                return 'err' \
            end } \
            xpcall(function() \
                error(setmetatable({}, mt)) \
            end, function(err) \
                collectgarbage('step') \
                local s = tostring(err) \
                for i=1,5 do local _={} end \
                return s \
            end) \
        end",
    ];
    let lua = variants[rng.random_range(0..variants.len())];
    emit_loadstring_call(b, lua, vec![], 0)?;
    Some(())
}

// -- unpack with large ranges ------------------------------------------------

fn gen_unpack_large_range(b: &mut ProgramBuilder, rng: &mut dyn RngCore) -> Option<()> {
    let boundaries: &[&str] = &["2147483647", "2147483646", "1000000", "-2147483648"];
    let hi = boundaries[rng.random_range(0..boundaries.len())];
    let lua = format!(
        "return function() \
            local t = {{}} \
            local ok, err = pcall(unpack, t, 1, {hi}) \
            collectgarbage('step') \
        end"
    );
    emit_loadstring_call(b, &lua, vec![], 0)?;
    Some(())
}

// -- Registry ----------------------------------------------------------------

pub fn all_generators() -> Vec<Generator> {
    let gens = vec![
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
            weight: 7.0,
            generate: gen_sort_comparator,
        },
        Generator {
            name: "gsub_callback",
            weight: 5.0,
            generate: gen_gsub_callback,
        },
        Generator {
            name: "metamethod_gc",
            weight: 8.0,
            generate: gen_metamethod_callback_with_gc,
        },
        Generator {
            name: "coroutine",
            weight: 3.0,
            generate: gen_coroutine_basic,
        },
        Generator {
            name: "upvalue",
            weight: 3.0,
            generate: gen_upvalue,
        },
        Generator {
            name: "error_throw",
            weight: 4.0,
            generate: gen_error_throw,
        },
        Generator {
            name: "pcall_wrap",
            weight: 4.0,
            generate: gen_pcall_wrap,
        },
        Generator {
            name: "property_remove",
            weight: 3.0,
            generate: gen_property_remove,
        },
        Generator {
            name: "reassign",
            weight: 2.0,
            generate: gen_reassign,
        },
        Generator {
            name: "metatable_swap",
            weight: 5.0,
            generate: gen_metatable_swap,
        },
        Generator {
            name: "table_resize",
            weight: 4.0,
            generate: gen_table_resize,
        },
        Generator {
            name: "weak_table",
            weight: 5.0,
            generate: gen_weak_table,
        },
        Generator {
            name: "stdlib_overwrite",
            weight: 3.0,
            generate: gen_stdlib_overwrite,
        },
        Generator {
            name: "coroutine_gc_interleave",
            weight: 5.0,
            generate: gen_coroutine_gc_interleave,
        },
        Generator {
            name: "method_call_via_table",
            weight: 5.0,
            generate: gen_method_call_via_table,
        },
        Generator {
            name: "compare_with_branch",
            weight: 5.0,
            generate: gen_compare_with_branch,
        },
        Generator {
            name: "custom_iterator",
            weight: 5.0,
            generate: gen_custom_iterator,
        },
        Generator {
            name: "metatable_hierarchy",
            weight: 5.0,
            generate: gen_metatable_hierarchy,
        },
        Generator {
            name: "table_populate",
            weight: 5.0,
            generate: gen_table_populate,
        },
        Generator {
            name: "mutation_during_iteration",
            weight: 7.0,
            generate: gen_mutation_during_iteration,
        },
        Generator {
            name: "xpcall_handler",
            weight: 7.0,
            generate: gen_xpcall_handler,
        },
        Generator {
            name: "string_format_tostring",
            weight: 7.0,
            generate: gen_string_format_tostring,
        },
        Generator {
            name: "table_concat_tostring",
            weight: 6.0,
            generate: gen_table_concat_tostring,
        },
        Generator {
            name: "deep_index_coroutine",
            weight: 5.0,
            generate: gen_deep_index_coroutine,
        },
        Generator {
            name: "error_tostring_gc",
            weight: 7.0,
            generate: gen_error_tostring_gc,
        },
        Generator {
            name: "unpack_large_range",
            weight: 5.0,
            generate: gen_unpack_large_range,
        },
    ];

    gens
}
