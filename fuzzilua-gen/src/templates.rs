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
    let count = rng.random_range(3..=8);
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

// -- WeakTableResurrection ---------------------------------------------------

pub struct WeakTableResurrection;

impl ProgramTemplate for WeakTableResurrection {
    fn name(&self) -> &'static str {
        "weak_table_resurrection"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Finalizers that resurrect objects by storing them in a strong table,
        // combined with weak table iteration during GC sweep.
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local strong = {} \
                local weak = setmetatable({}, {__mode='v'}) \
                for i=1,8 do \
                    local p = newproxy(true) \
                    getmetatable(p).__gc = function(self) strong[#strong+1] = self end \
                    weak[i] = p \
                end \
                collectgarbage('collect') \
                collectgarbage('collect') \
                for k,v in pairs(weak) do collectgarbage('step') end \
            end"
            }
            1 => {
                "return function() \
                local weak_k = setmetatable({}, {__mode='k'}) \
                local weak_v = setmetatable({}, {__mode='v'}) \
                local anchor = {} \
                for i=1,10 do \
                    local t = setmetatable({}, {__gc=function() collectgarbage('step') end}) \
                    weak_k[t] = i \
                    weak_v[i] = t \
                    if i % 2 == 0 then anchor[i] = t end \
                end \
                anchor = nil \
                collectgarbage('collect') \
                for k,v in pairs(weak_k) do end \
                collectgarbage('collect') \
            end"
            }
            _ => {
                "return function() \
                local ephemeron = setmetatable({}, {__mode='k'}) \
                local keys = {} \
                for i=1,6 do \
                    local k = {} \
                    ephemeron[k] = setmetatable({}, {__gc=function() collectgarbage('step') end}) \
                    keys[i] = k \
                end \
                for i=1,3 do keys[i] = nil end \
                collectgarbage('collect') \
                collectgarbage('collect') \
                for k,v in pairs(ephemeron) do end \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        b.emit(Op::CollectGarbage(GcMode::Collect), vec![]);
        fill_random(b, generators, rng);
    }
}

// -- UpvalueSharing ----------------------------------------------------------

pub struct UpvalueSharing;

impl ProgramTemplate for UpvalueSharing {
    fn name(&self) -> &'static str {
        "upvalue_sharing"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Multiple closures sharing the same upvalue, with mutation and GC
        // between calls. Stresses open→closed upvalue transitions.
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local val = {data='hello'} \
                local readers = {} \
                local writers = {} \
                for i=1,5 do \
                    readers[i] = function() collectgarbage('step'); return val end \
                    writers[i] = function(v) val = v; collectgarbage('step') end \
                end \
                collectgarbage('collect') \
                for i=1,5 do \
                    writers[i]({n=i}) \
                    collectgarbage('step') \
                    local _ = readers[(i%5)+1]() \
                end \
            end"
            }
            1 => {
                "return function() \
                local function make_chain(n) \
                    local captured = {} \
                    local fns = {} \
                    for i=1,n do \
                        captured[i] = {v=i} \
                        local prev = captured \
                        fns[i] = function() \
                            collectgarbage('step') \
                            return prev[i] \
                        end \
                    end \
                    return fns, captured \
                end \
                local fns, caps = make_chain(8) \
                caps = nil \
                collectgarbage('collect') \
                for i=1,8 do pcall(fns[i]) end \
            end"
            }
            _ => {
                "return function() \
                local x = 1 \
                local co = coroutine.create(function() \
                    for i=1,5 do x = x + 1; coroutine.yield(x) end \
                end) \
                local function read() return x end \
                for i=1,5 do \
                    coroutine.resume(co) \
                    collectgarbage('step') \
                    local _ = read() \
                end \
                collectgarbage('collect') \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- TableRehash -------------------------------------------------------------

pub struct TableRehash;

impl ProgramTemplate for TableRehash {
    fn name(&self) -> &'static str {
        "table_rehash"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Force hash table rehashing with GC pressure. Exercises luaH_resize,
        // array/hash boundary transitions.
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local t = {} \
                for i=1,64 do \
                    t[tostring(i)] = {} \
                    if i % 8 == 0 then collectgarbage('step') end \
                end \
                for i=1,32 do t[tostring(i)] = nil end \
                collectgarbage('collect') \
                for i=65,96 do t[tostring(i)] = {} end \
                for k,v in pairs(t) do collectgarbage('step') end \
            end"
            }
            1 => {
                "return function() \
                local t = {} \
                for i=1,32 do t[i] = i end \
                for i=1,32 do t['k'..i] = {} end \
                collectgarbage('step') \
                for i=1,16 do t[i] = nil end \
                for i=33,48 do t[i] = {} end \
                collectgarbage('collect') \
                local n = 0 \
                for k,v in next, t do n = n+1; if n%4==0 then collectgarbage('step') end end \
            end"
            }
            _ => {
                "return function() \
                local t = {} \
                local mt = {__newindex = function(self, k, v) \
                    collectgarbage('step') \
                    rawset(self, k, v) \
                end} \
                setmetatable(t, mt) \
                for i=1,48 do t['field'..i] = {i} end \
                collectgarbage('collect') \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- VarargStress ------------------------------------------------------------

pub struct VarargStress;

impl ProgramTemplate for VarargStress {
    fn name(&self) -> &'static str {
        "vararg_stress"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local function collect(...) \
                    collectgarbage('step') \
                    local t = {...} \
                    for i=1,select('#', ...) do t[i] = tostring(t[i]) end \
                    return unpack(t) \
                end \
                local function make() return {},{},{},{},{} end \
                local a,b,c,d,e = collect(make()) \
                collectgarbage('collect') \
                local _ = collect(a,b,c,d,e) \
            end"
            }
            1 => {
                "return function() \
                local function tail(...) \
                    if select('#', ...) > 1 then \
                        collectgarbage('step') \
                        return tail(select(2, ...)) \
                    end \
                    return ... \
                end \
                local r = tail({},{},{},{},{},{},{},{}) \
                collectgarbage('collect') \
            end"
            }
            _ => {
                "return function() \
                local function pack_gc(...) \
                    collectgarbage('step') \
                    return {n=select('#',...),...} \
                end \
                local results = {} \
                for i=1,5 do \
                    results[i] = pack_gc(string.rep('x',i), {}, true, i) \
                end \
                collectgarbage('collect') \
                for i=1,5 do \
                    local t = results[i] \
                    for j=1,t.n do local _ = t[j] end \
                end \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- ErrorUnwindGc -----------------------------------------------------------

pub struct ErrorUnwindGc;

impl ProgramTemplate for ErrorUnwindGc {
    fn name(&self) -> &'static str {
        "error_unwind_gc"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Error unwinding with active finalizers and __tostring metamethods.
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local mt = {__tostring = function() collectgarbage('step'); return 'err' end} \
                local err_obj = setmetatable({}, mt) \
                local function inner() \
                    local p = newproxy(true) \
                    getmetatable(p).__gc = function() collectgarbage('step') end \
                    error(err_obj) \
                end \
                local ok, e = xpcall(inner, function(err) \
                    collectgarbage('collect') \
                    return tostring(err) .. ' handled' \
                end) \
            end"
            }
            1 => {
                "return function() \
                local depth = 0 \
                local function recur() \
                    depth = depth + 1 \
                    local t = {} \
                    if depth < 5 then \
                        local ok, err = pcall(recur) \
                        collectgarbage('step') \
                        if not ok then error(err, 0) end \
                    else \
                        error(setmetatable({}, {__tostring=function() return 'deep' end})) \
                    end \
                end \
                pcall(recur) \
                collectgarbage('collect') \
            end"
            }
            _ => {
                "return function() \
                local live = {} \
                for i=1,5 do \
                    local ok = pcall(function() \
                        live[i] = setmetatable({}, {__gc=function() collectgarbage('step') end}) \
                        if i == 3 then error('boom') end \
                    end) \
                end \
                live = nil \
                collectgarbage('collect') \
                collectgarbage('collect') \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- DebugHookGc -------------------------------------------------------------

pub struct DebugHookGc;

impl ProgramTemplate for DebugHookGc {
    fn name(&self) -> &'static str {
        "debug_hook_gc"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // Debug hooks with GC inside — exercises stack manipulation + collection.
        let lua = match rng.random_range(0..2u8) {
            0 => {
                "return function() \
                local allocs = {} \
                debug.sethook(function(event, line) \
                    allocs[#allocs+1] = {} \
                    if #allocs % 3 == 0 then collectgarbage('step') end \
                end, 'l', 1) \
                local t = {} \
                for i=1,20 do t[i] = string.rep('x', i) end \
                debug.sethook() \
                collectgarbage('collect') \
            end"
            }
            _ => {
                "return function() \
                local count = 0 \
                debug.sethook(function() \
                    count = count + 1 \
                    if count % 5 == 0 then \
                        local info = debug.getinfo(2, 'nSl') \
                        collectgarbage('step') \
                    end \
                end, '', 10) \
                local function work() \
                    local t = {} \
                    for i=1,30 do t[i] = {v=i} end \
                    return t \
                end \
                pcall(work) \
                debug.sethook() \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- StringPatternGc ---------------------------------------------------------

pub struct StringPatternGc;

impl ProgramTemplate for StringPatternGc {
    fn name(&self) -> &'static str {
        "string_pattern_gc"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        // String pattern matching with GC — exercises string creation/interning.
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local s = string.rep('abcdef', 20) \
                local parts = {} \
                for w in string.gmatch(s, '(%a+)') do \
                    parts[#parts+1] = w \
                    if #parts % 5 == 0 then collectgarbage('step') end \
                end \
                collectgarbage('collect') \
                local r = string.gsub(s, '(%a)(%a)', function(a,b) \
                    collectgarbage('step'); return b..a \
                end) \
            end"
            }
            1 => {
                "return function() \
                local results = {} \
                for i=1,10 do \
                    local s = string.rep(string.char(96+i), i*5) \
                    local a,b = string.find(s, string.rep('.', i)) \
                    results[i] = {a, b, s} \
                    collectgarbage('step') \
                end \
                collectgarbage('collect') \
            end"
            }
            _ => {
                "return function() \
                local s = '' \
                for i=1,50 do s = s .. string.char(32 + (i % 95)) end \
                local count = 0 \
                string.gsub(s, '(.)', function(c) \
                    count = count + 1 \
                    if count % 10 == 0 then collectgarbage('step') end \
                    return string.upper(c) \
                end) \
                collectgarbage('collect') \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- NestedCoroutineYield ----------------------------------------------------

pub struct NestedCoroutineYield;

impl ProgramTemplate for NestedCoroutineYield {
    fn name(&self) -> &'static str {
        "nested_coroutine_yield"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let lua = match rng.random_range(0..3u8) {
            0 => {
                "return function() \
                local function inner() \
                    for i=1,3 do \
                        local t = setmetatable({}, {__gc=function() collectgarbage('step') end}) \
                        coroutine.yield(t) \
                    end \
                end \
                local function outer() \
                    local co2 = coroutine.create(inner) \
                    for i=1,3 do \
                        local ok, val = coroutine.resume(co2) \
                        coroutine.yield(val) \
                    end \
                end \
                local co = coroutine.create(outer) \
                for i=1,3 do \
                    local ok, val = coroutine.resume(co) \
                    collectgarbage('collect') \
                end \
            end"
            }
            1 => {
                "return function() \
                local wrap = coroutine.wrap(function() \
                    local t = {} \
                    for i=1,6 do \
                        t[i] = setmetatable({}, { \
                            __index = function(self, k) \
                                coroutine.yield(k) \
                                return rawget(self, k) \
                            end \
                        }) \
                    end \
                    for i=1,6 do local _ = t[i].missing end \
                end) \
                for i=1,6 do \
                    local v = wrap() \
                    collectgarbage('step') \
                end \
            end"
            }
            _ => {
                "return function() \
                local co = coroutine.create(function() \
                    local function deep(n) \
                        if n <= 0 then coroutine.yield() return end \
                        local t = {} \
                        deep(n-1) \
                        collectgarbage('step') \
                    end \
                    deep(8) \
                end) \
                coroutine.resume(co) \
                collectgarbage('collect') \
                coroutine.resume(co) \
            end"
            }
        };
        emit_loadstring_call(b, lua, vec![], 0);
        fill_random(b, generators, rng);
    }
}

// -- MathCoercionGc ----------------------------------------------------------

pub struct MathCoercionGc;

impl ProgramTemplate for MathCoercionGc {
    fn name(&self) -> &'static str {
        "math_coercion_gc"
    }

    fn generate(&self, b: &mut ProgramBuilder, generators: &[Generator], rng: &mut dyn RngCore) {
        let lua = "return function() \
            local vals = {'123', '45.6', '0', '-1', '1e10', '0x1A'} \
            local ops = {} \
            for i=1,#vals do \
                ops[i] = tonumber(vals[i]) \
                collectgarbage('step') \
            end \
            local mt = {__add=function(a,b) collectgarbage('step'); return 0 end, \
                        __lt=function(a,b) collectgarbage('step'); return true end, \
                        __eq=function(a,b) collectgarbage('step'); return false end} \
            local t = setmetatable({}, mt) \
            for i=1,#ops do \
                pcall(function() local _ = t + ops[i] end) \
                pcall(function() local _ = t < ops[i] end) \
                pcall(function() local _ = math.sin(ops[i] or 0) end) \
            end \
            collectgarbage('collect') \
        end";
        emit_loadstring_call(b, lua, vec![], 0);
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
        Box::new(WeakTableResurrection),
        Box::new(UpvalueSharing),
        Box::new(TableRehash),
        Box::new(VarargStress),
        Box::new(ErrorUnwindGc),
        Box::new(DebugHookGc),
        Box::new(StringPatternGc),
        Box::new(NestedCoroutineYield),
        Box::new(MathCoercionGc),
    ]
}
