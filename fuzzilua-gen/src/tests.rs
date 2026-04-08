use crate::builder::ProgramBuilder;
use crate::{all_generators, all_templates, generate_program};
use fuzzilua_ir::{LuaType, Op, lift};
use rand::SeedableRng;
use rand::rngs::StdRng;
use test_case::test_case;

const FUZZ_SEED: u64 = 0xDEAD_BEEF_CAFE;
const FUZZ_ITERATIONS: usize = 1_000;

// -- ProgramBuilder tests ----------------------------------------------------

#[test]
fn builder_depth_limit() {
    let mut b = ProgramBuilder::new(100, 3);

    for i in 0..3 {
        let cond = b.emit(Op::LoadBool(true), vec![]).unwrap();
        assert!(
            b.begin_block(Op::BeginIf, vec![cond[0]]).is_some(),
            "block {i} should succeed"
        );
    }

    let cond = b.emit(Op::LoadBool(true), vec![]).unwrap();
    assert!(b.begin_block(Op::BeginIf, vec![cond[0]]).is_none());

    assert!(b.finish().validate().is_ok());
}

#[test]
fn builder_budget_exhaustion() {
    let mut b = ProgramBuilder::new(5, 5);
    for _ in 0..5 {
        assert!(b.emit(Op::LoadNil, vec![]).is_some());
    }
    assert!(b.emit(Op::LoadNil, vec![]).is_none());
}

#[test]
fn builder_scope_visibility() {
    let mut b = ProgramBuilder::new(100, 5);
    let mut rng = StdRng::seed_from_u64(42);

    let outer = b.emit(Op::LoadInt(1), vec![]).unwrap();

    b.emit(Op::CreateTable, vec![]).unwrap();
    b.emit(Op::LoadInt(42), vec![]).unwrap();
    assert!(
        b.random_variable_of_type(LuaType::Table, &mut rng)
            .is_some()
    );
    assert!(
        b.random_variable_of_type(LuaType::Integer, &mut rng)
            .is_some()
    );
    assert!(
        b.random_variable_of_type(LuaType::Coroutine, &mut rng)
            .is_none()
    );

    let cond = b.emit(Op::LoadBool(true), vec![]).unwrap();
    b.begin_block(Op::BeginIf, vec![cond[0]]).unwrap();
    let inner = b.emit(Op::LoadInt(2), vec![]).unwrap();
    assert!(b.visible_variables().count() >= 2);

    b.end_block();
    let vars: Vec<_> = b.visible_variables().collect();
    assert!(vars.iter().any(|(v, _)| *v == outer[0]));
    assert!(!vars.iter().any(|(v, _)| *v == inner[0]));

    assert!(b.finish().validate().is_ok());
}

#[test]
fn builder_rejects_begin_else_as_block_opener() {
    let mut b = ProgramBuilder::new(100, 5);
    assert!(b.begin_block(Op::BeginElse, vec![]).is_none());
}

// -- Per-generator smoke tests -----------------------------------------------
//
// Each generator must: (a) produce valid IR, (b) lift to non-empty Lua.
// We don't assert on specific Lua substrings because that couples these tests
// to lifter formatting. The fuzz harness below covers structural invariants.

fn run_generator_by_name(name: &str) -> (fuzzilua_ir::Program, String) {
    let generators = all_generators();
    let g = generators
        .iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("unknown generator: {name}"));
    let mut b = ProgramBuilder::new(200, 5);
    let mut rng = StdRng::seed_from_u64(42);
    (g.generate)(&mut b, &mut rng);
    let prog = b.finish();
    let lua = lift(&prog);
    assert!(
        prog.validate().is_ok(),
        "generator {name} produced invalid IR"
    );
    assert!(!lua.is_empty(), "generator {name} lifted to empty Lua");
    (prog, lua)
}

#[test_case("int")]
#[test_case("float")]
#[test_case("string")]
#[test_case("bool")]
#[test_case("nil")]
#[test_case("create_table")]
#[test_case("get_property")]
#[test_case("set_property")]
#[test_case("__index")]
#[test_case("__newindex")]
#[test_case("__eq")]
#[test_case("__concat")]
#[test_case("__len")]
#[test_case("__add")]
#[test_case("__call")]
#[test_case("gc_collect")]
#[test_case("gc_step")]
#[test_case("begin_function")]
#[test_case("call_function")]
#[test_case("if")]
#[test_case("while")]
#[test_case("for_numeric")]
#[test_case("for_generic")]
#[test_case("sort_comparator")]
#[test_case("gsub_callback")]
#[test_case("metamethod_gc")]
#[test_case("coroutine")]
#[test_case("loadstring")]
#[test_case("upvalue")]
#[test_case("alloc_pressure")]
fn generator_produces_valid_ir(name: &str) {
    run_generator_by_name(name);
}

// -- Fuzz harness ------------------------------------------------------------

#[test]
fn fuzz_generate_program() {
    let generators = all_generators();
    let mut rng = StdRng::seed_from_u64(FUZZ_SEED);
    let budgets = [10, 50, 200, 500];

    for i in 0..FUZZ_ITERATIONS {
        let budget = budgets[i % budgets.len()];
        let prog = generate_program(&mut rng, budget, 5, &generators);

        if let Err(errors) = prog.validate() {
            panic!("program {i} (budget={budget}) failed validation: {errors:?}\n{prog:?}");
        }

        let lua = lift(&prog);
        assert!(
            !lua.is_empty() || prog.instructions.iter().all(|i| matches!(i.op, Op::Nop)),
            "program {i}: lifted to empty string but has non-Nop instructions"
        );

        assert!(
            prog.instructions.len() <= budget,
            "program {i}: {} instructions exceeds budget {budget}",
            prog.instructions.len()
        );

        let mut depth = 0usize;
        let mut max_observed = 0usize;
        for instr in &prog.instructions {
            if instr.op.opens_block().is_some() {
                depth += 1;
                max_observed = max_observed.max(depth);
            }
            if instr.op.closes_block().is_some() {
                depth = depth.saturating_sub(1);
            }
        }
        assert!(
            max_observed <= 5,
            "program {i}: nesting depth {max_observed} exceeds max 5"
        );
    }
}

// -- Template tests ----------------------------------------------------------

#[test_case("metamethod_stress")]
#[test_case("parse_reentry")]
#[test_case("sort_exploit")]
#[test_case("gsub_reentry")]
#[test_case("concat_chain")]
#[test_case("coroutine_gc")]
#[test_case("cjson_metamethod")]
#[test_case("metatable_nesting")]
#[test_case("upvalue_lifetime")]
fn template_produces_valid_ir(name: &str) {
    let templates = all_templates();
    let generators = all_generators();
    let t = templates
        .iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("unknown template: {name}"));
    let mut b = ProgramBuilder::new(200, 5);
    let mut rng = StdRng::seed_from_u64(42);
    t.generate(&mut b, &generators, &mut rng);
    let prog = b.finish();
    let lua = lift(&prog);
    assert!(
        prog.validate().is_ok(),
        "template {name} produced invalid IR"
    );
    assert!(!lua.is_empty(), "template {name} lifted to empty Lua");
}

#[test]
fn each_template_100_valid_programs() {
    let templates = all_templates();
    let generators = all_generators();
    let budgets = [20, 50, 200, 500];

    for template in &templates {
        for seed in 0..100u64 {
            let budget = budgets[seed as usize % budgets.len()];
            let mut rng = StdRng::seed_from_u64(seed);
            let mut b = ProgramBuilder::new(budget, 5);
            template.generate(&mut b, &generators, &mut rng);
            let prog = b.finish();
            if let Err(errors) = prog.validate() {
                panic!(
                    "template '{}' budget={budget} seed={seed} failed validation: {errors:?}",
                    template.name()
                );
            }
            let lua = lift(&prog);
            assert!(
                !lua.is_empty(),
                "template '{}' seed={seed} lifted to empty Lua",
                template.name()
            );
        }
    }
}
