use crate::builder::ProgramBuilder;
use crate::generators::{Generator, emit_loadstring_call, trigger_metamethod};
use crate::pick_weighted;
use fuzzilua_ir::{BinOp, GcMode, METAMETHODS, Op};
use rand::{Rng, RngCore};

pub trait ProgramTemplate: Send + Sync {
    fn name(&self) -> &'static str;
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        generators: &[Generator],
        rng: &mut dyn RngCore,
    );
}

pub(crate) fn fill_random(
    builder: &mut ProgramBuilder,
    generators: &[Generator],
    rng: &mut dyn RngCore,
) {
    let total_weight: f64 = generators.iter().map(|g| g.weight).sum();
    let count = rng.random_range(1..=3);
    for _ in 0..count {
        if builder.remaining_budget() == 0 {
            return;
        }
        if let Some(g) = pick_weighted(rng, generators, total_weight) {
            (g.generate)(builder, rng);
        }
    }
}

// -- MetamethodStress --------------------------------------------------------

pub struct MetamethodStress;

impl ProgramTemplate for MetamethodStress {
    fn name(&self) -> &'static str {
        "metamethod_stress"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let Some(target) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
            return;
        };
        let Some(mt) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
            return;
        };

        let count = rng.random_range(3..=5);
        let mut installed = Vec::new();

        for _ in 0..count {
            if b.remaining_budget() < 4 {
                break;
            }
            let entry = &METAMETHODS[rng.random_range(0..METAMETHODS.len())];
            let mm = entry.name;
            let pc = entry.param_count;

            if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: pc }, vec![]) {
                let fn_var = fn_out[0];
                b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
                if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
                    b.emit(Op::Return, vec![v]);
                }
                b.end_block();
                b.emit(Op::TableSetField(mm.into()), vec![mt, fn_var]);
                installed.push(mm);
            }
        }

        b.emit(Op::SetMetatable, vec![target, mt]);

        for mm in &installed {
            if b.remaining_budget() < 2 {
                break;
            }
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
            trigger_metamethod(b, rng, target, mm);
        }

        fill_random(b, generators, rng);
    }
}

// -- ParseReentry ------------------------------------------------------------

pub struct ParseReentry;

impl ProgramTemplate for ParseReentry {
    fn name(&self) -> &'static str {
        "parse_reentry"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let lua = "return function() local t = {} for i=1,10 do t[i] = string.rep('x', i) end return t end";
        let Some(code) = b.emit(Op::LoadString(lua.into()), vec![]).map(|v| v[0]) else {
            return;
        };
        let Some(loader) = b.emit(Op::Loadstring, vec![code]).map(|v| v[0]) else {
            return;
        };
        let Some(factory) = b.emit_n(
            Op::CallFunction {
                arg_count: 0,
                ret_count: 1,
            },
            vec![loader],
            1,
        ) else {
            return;
        };
        let f = factory[0];

        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);

        b.emit_n(
            Op::CallFunction {
                arg_count: 0,
                ret_count: 1,
            },
            vec![f],
            1,
        );

        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);

        fill_random(b, generators, rng);
    }
}

// -- SortExploit -------------------------------------------------------------

pub struct SortExploit;

impl ProgramTemplate for SortExploit {
    fn name(&self) -> &'static str {
        "sort_exploit"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let Some(t) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
            return;
        };

        let count = rng.random_range(16..=64).min(b.remaining_budget() / 2);
        for i in 1..=count {
            let Some(v) = b
                .emit(Op::LoadInt(rng.random_range(0..1000)), vec![])
                .map(|v| v[0])
            else {
                return;
            };
            if b.emit(Op::TableSetNumericField(i as i64), vec![t, v])
                .is_none()
            {
                return;
            }
        }

        let lua = "return function(t) table.sort(t, function(a,b) collectgarbage('step') return a < b end) end";
        emit_loadstring_call(b, lua, vec![t], 0);

        fill_random(b, generators, rng);
    }
}

// -- GsubReentry -------------------------------------------------------------

pub struct GsubReentry;

impl ProgramTemplate for GsubReentry {
    fn name(&self) -> &'static str {
        "gsub_reentry"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let Some(s) = b.ensure_string(rng) else {
            return;
        };

        let lua = "return function(s) return string.gsub(s, '.', function(c) collectgarbage('step'); local t = {} for i=1,5 do t[i]=string.rep('y',i) end return c end) end";
        emit_loadstring_call(b, lua, vec![s], 1);

        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);

        fill_random(b, generators, rng);
    }
}

// -- ConcatChain -------------------------------------------------------------

pub struct ConcatChain;

impl ProgramTemplate for ConcatChain {
    fn name(&self) -> &'static str {
        "concat_chain"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let chain_len = rng.random_range(5..=20).min(b.remaining_budget() / 6);
        if chain_len < 2 {
            return;
        }

        let mut vars = Vec::with_capacity(chain_len);
        for _ in 0..chain_len {
            if b.remaining_budget() < 5 {
                break;
            }

            let Some(t) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
                break;
            };
            let Some(mt) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
                break;
            };

            if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 2 }, vec![]) {
                let fn_var = fn_out[0];
                b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
                if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
                    b.emit(Op::Return, vec![v]);
                }
                b.end_block();
                b.emit(Op::TableSetField("__concat".into()), vec![mt, fn_var]);
            }

            b.emit(Op::SetMetatable, vec![t, mt]);
            vars.push(t);
        }

        if vars.len() >= 2 {
            let mut acc = vars[0];
            for &v in &vars[1..] {
                if b.remaining_budget() < 2 {
                    break;
                }
                if let Some(result) = b
                    .emit(Op::BinaryOp(BinOp::Concat), vec![acc, v])
                    .map(|v| v[0])
                {
                    acc = result;
                }
                b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
            }
        }

        fill_random(b, generators, rng);
    }
}

// -- CoroutineGc -------------------------------------------------------------

pub struct CoroutineGc;

impl ProgramTemplate for CoroutineGc {
    fn name(&self) -> &'static str {
        "coroutine_gc"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 0 }, vec![]) else {
            return;
        };
        let fn_var = fn_out[0];

        // Body: yield in a loop
        let Some(limit) = b
            .emit(Op::LoadInt(rng.random_range(3..=8)), vec![])
            .map(|v| v[0])
        else {
            b.end_block();
            return;
        };
        let Some(start) = b.emit(Op::LoadInt(1), vec![]).map(|v| v[0]) else {
            b.end_block();
            return;
        };
        if b.begin_block(Op::BeginForRange, vec![start, limit])
            .is_some()
        {
            b.emit(Op::CreateTable, vec![]);
            b.emit(Op::CoroutineYield, vec![]);
            b.end_block();
        }
        b.end_block();

        let Some(co) = b.emit(Op::CoroutineCreate, vec![fn_var]).map(|v| v[0]) else {
            return;
        };

        // Resume loop with GC between each resume
        let iterations = rng.random_range(3..=6).min(b.remaining_budget() / 3);
        for _ in 0..iterations {
            if b.remaining_budget() < 2 {
                break;
            }
            b.emit_n(Op::CoroutineResume, vec![co], 1);
            b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
        }

        fill_random(b, generators, rng);
    }
}

// -- CjsonMetamethod ---------------------------------------------------------

pub struct CjsonMetamethod;

impl ProgramTemplate for CjsonMetamethod {
    fn name(&self) -> &'static str {
        "cjson_metamethod"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let Some(t) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
            return;
        };
        let Some(mt) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
            return;
        };

        // __tostring metamethod with GC
        if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 1 }, vec![]) {
            let fn_var = fn_out[0];
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
            let Some(s) = b
                .emit(Op::LoadString("json_val".into()), vec![])
                .map(|v| v[0])
            else {
                b.end_block();
                b.emit(Op::TableSetField("__tostring".into()), vec![mt, fn_var]);
                b.emit(Op::SetMetatable, vec![t, mt]);
                return;
            };
            b.emit(Op::Return, vec![s]);
            b.end_block();
            b.emit(Op::TableSetField("__tostring".into()), vec![mt, fn_var]);
        }

        // __index metamethod
        if let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 2 }, vec![]) {
            let fn_var = fn_out[0];
            b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
            if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
                b.emit(Op::Return, vec![v]);
            }
            b.end_block();
            b.emit(Op::TableSetField("__index".into()), vec![mt, fn_var]);
        }

        b.emit(Op::SetMetatable, vec![t, mt]);

        // Populate a field so cjson has something to encode
        if let Some(v) = b.emit(Op::LoadInt(42), vec![]).map(|v| v[0]) {
            b.emit(Op::TableSetField("x".into()), vec![t, v]);
        }

        let lua = "return function(t) \
            if pcall(require, 'cjson') then \
                local ok, s = pcall(require('cjson').encode, t) \
                if ok and s then pcall(require('cjson').decode, s) end \
            end \
        end";
        emit_loadstring_call(b, lua, vec![t], 0);

        fill_random(b, generators, rng);
    }
}

// -- MetatableNesting --------------------------------------------------------

pub struct MetatableNesting;

impl ProgramTemplate for MetatableNesting {
    fn name(&self) -> &'static str {
        "metatable_nesting"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let depth = rng.random_range(5..=10).min(b.remaining_budget() / 4);
        if depth < 2 {
            return;
        }

        let mut tables = Vec::with_capacity(depth);
        for _ in 0..depth {
            let Some(t) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
                break;
            };
            tables.push(t);
        }
        if tables.len() < 2 {
            return;
        }

        // Chain: t[i].__index = t[i+1], setmetatable(t[i], mt_i)
        for i in 0..tables.len() - 1 {
            if b.remaining_budget() < 4 {
                break;
            }
            let Some(mt) = b.emit(Op::CreateTable, vec![]).map(|v| v[0]) else {
                break;
            };
            b.emit(Op::TableSetField("__index".into()), vec![mt, tables[i + 1]]);
            b.emit(Op::SetMetatable, vec![tables[i], mt]);
            b.emit(Op::CollectGarbage(GcMode::Step), vec![]);
        }

        // Put a value at the deepest table
        let last = *tables.last().unwrap();
        if let Some(v) = b.emit(Op::LoadInt(999), vec![]).map(|v| v[0]) {
            b.emit(Op::TableSetField("deep".into()), vec![last, v]);
        }

        // Access from the top to trigger full chain walk
        let first = tables[0];
        b.emit(Op::TableGetField("deep".into()), vec![first]);
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);

        fill_random(b, generators, rng);
    }
}

// -- UpvalueLifetime ---------------------------------------------------------

pub struct UpvalueLifetime;

impl ProgramTemplate for UpvalueLifetime {
    fn name(&self) -> &'static str {
        "upvalue_lifetime"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Create several locals as upvalue candidates
        let local_count = rng.random_range(2..=5).min(b.remaining_budget() / 4);
        let mut locals = Vec::with_capacity(local_count);
        for _ in 0..local_count {
            let Some(v) = b
                .emit(Op::LoadInt(rng.random_range(0..100)), vec![])
                .map(|v| v[0])
            else {
                break;
            };
            locals.push(v);
        }
        if locals.is_empty() {
            return;
        }

        // Define function that captures them
        let Some(fn_out) = b.begin_block(Op::BeginFunction { param_count: 0 }, vec![]) else {
            return;
        };
        let fn_var = fn_out[0];
        for &local in &locals {
            b.emit(Op::Reassign, vec![local]);
        }
        if let Some(v) = b.random_variable_of_type(fuzzilua_ir::LuaType::Anything, rng) {
            b.emit(Op::Return, vec![v]);
        }
        b.end_block();

        // GC before calling — stress upvalue lifetime
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
        b.emit(Op::CollectGarbage(GcMode::Step), vec![]);

        b.emit_n(
            Op::CallFunction {
                arg_count: 0,
                ret_count: 1,
            },
            vec![fn_var],
            1,
        );

        fill_random(b, generators, rng);
    }
}

// -- Registry ----------------------------------------------------------------

pub fn all_templates() -> Vec<Box<dyn ProgramTemplate>> {
    vec![
        Box::new(MetamethodStress),
        Box::new(ParseReentry),
        Box::new(SortExploit),
        Box::new(GsubReentry),
        Box::new(ConcatChain),
        Box::new(CoroutineGc),
        Box::new(CjsonMetamethod),
        Box::new(MetatableNesting),
        Box::new(UpvalueLifetime),
    ]
}
