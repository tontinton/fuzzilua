use fuzzilua_gen::{all_generators, generate_program};
use fuzzilua_ir::{BinOp, CmpOp, Op, Program, Variable};
use rand::SeedableRng;
use rand::rngs::StdRng;
use test_case::test_case;

use crate::engine::MutationEngine;
use crate::hybrid::HybridEngine;
use crate::mutators::{
    CallbackGcMutator, ChainDepthMutator, CodeGenMutator, CombineMutator, GcInjectionMutator,
    InputMutator, InterleaveMutator, LoadstringWrapMutator, MetamethodSwapMutator, Mutator,
    OperationMutator, SpliceMutator, TableSizeMutator,
};

const FUZZ_SEED: u64 = 0xCAFE_1234_5678;
const FUZZ_PROGRAMS: usize = 500;
const MUTATIONS_PER_PROGRAM: usize = 50;
const MAX_PROGRAM_SIZE: usize = 150;

fn make_three_var_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.emit(Op::LoadString("hello".into()), vec![], vec![Variable(1)]);
    p.emit(Op::CreateTable, vec![], vec![Variable(2)]);
    p.emit(
        Op::BinaryOp(BinOp::Add),
        vec![Variable(0), Variable(0)],
        vec![Variable(3)],
    );
    p.next_var = 4;
    p
}

fn make_block_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::LoadBool(true), vec![], vec![Variable(0)]);
    p.emit(Op::BeginIf, vec![Variable(0)], vec![]);
    p.emit(Op::LoadInt(1), vec![], vec![Variable(1)]);
    p.emit(Op::EndIf, vec![], vec![]);
    p.emit(Op::LoadInt(2), vec![], vec![Variable(2)]);
    p.next_var = 3;
    p
}

fn make_metamethod_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::CreateTable, vec![], vec![Variable(0)]);
    p.emit(Op::CreateTable, vec![], vec![Variable(1)]);
    p.emit(Op::LoadInt(42), vec![], vec![Variable(2)]);
    p.emit(
        Op::TableSetField("__index".into()),
        vec![Variable(1), Variable(2)],
        vec![],
    );
    p.emit(
        Op::SetMetatable,
        vec![Variable(0), Variable(1)],
        vec![Variable(3)],
    );
    p.next_var = 4;
    p
}

fn make_table_populate_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::CreateTable, vec![], vec![Variable(0)]);
    let mut next = 1u32;
    for i in 1..=10 {
        let v = Variable(next);
        p.emit(Op::LoadInt(i * 10), vec![], vec![v]);
        next += 1;
        p.emit(Op::TableSetNumericField(i), vec![Variable(0), v], vec![]);
    }
    p.next_var = next;
    p
}

fn make_callback_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::CreateTable, vec![], vec![Variable(0)]);
    p.emit(
        Op::BeginFunction { param_count: 1 },
        vec![],
        vec![Variable(1), Variable(2)],
    );
    p.emit(Op::EndFunction, vec![], vec![]);
    p.emit(
        Op::BeginFunction { param_count: 2 },
        vec![],
        vec![Variable(3), Variable(4), Variable(5)],
    );
    p.emit(
        Op::Compare(CmpOp::Lt),
        vec![Variable(4), Variable(5)],
        vec![Variable(6)],
    );
    p.emit(Op::Return, vec![Variable(6)], vec![]);
    p.emit(Op::EndFunction, vec![], vec![]);
    p.emit(
        Op::CallFunction {
            arg_count: 2,
            ret_count: 0,
        },
        vec![Variable(1), Variable(0), Variable(3)],
        vec![],
    );
    p.next_var = 7;
    p
}

fn assert_mutator_valid(mutator: &dyn Mutator, programs: &[Program], iterations: usize) {
    let mut rng = StdRng::seed_from_u64(42);
    let mut applied = 0;
    for _ in 0..iterations {
        for p in programs {
            let mut candidate = p.clone();
            if mutator.mutate(&mut candidate, &mut rng) {
                assert!(
                    candidate.validate().is_ok(),
                    "{} produced invalid IR: {:?}",
                    mutator.name(),
                    candidate.validate().unwrap_err()
                );
                applied += 1;
            }
        }
    }
    assert!(applied > 0, "{} never applied", mutator.name());
}

fn mutator_by_name(name: &str) -> (Box<dyn Mutator>, Vec<Program>) {
    match name {
        "input" => (Box::new(InputMutator), vec![make_three_var_program()]),
        "operation" => (
            Box::new(OperationMutator),
            vec![make_three_var_program(), make_metamethod_program()],
        ),
        "splice" => (
            Box::new(SpliceMutator),
            vec![make_three_var_program(), make_block_program()],
        ),
        "combine" => (Box::new(CombineMutator), vec![make_three_var_program()]),
        "codegen" => (Box::new(CodeGenMutator), vec![make_three_var_program()]),
        "gc_injection" => (
            Box::new(GcInjectionMutator),
            vec![make_three_var_program(), make_block_program()],
        ),
        "chain_depth" => (Box::new(ChainDepthMutator), vec![make_metamethod_program()]),
        "table_size" => (
            Box::new(TableSizeMutator),
            vec![make_table_populate_program()],
        ),
        "interleave" => (
            Box::new(InterleaveMutator),
            vec![make_three_var_program(), make_block_program()],
        ),
        "loadstring_wrap" => (
            Box::new(LoadstringWrapMutator),
            vec![make_three_var_program()],
        ),
        "metamethod_swap" => (
            Box::new(MetamethodSwapMutator),
            vec![make_metamethod_program()],
        ),
        "callback_gc" => (Box::new(CallbackGcMutator), vec![make_callback_program()]),
        _ => panic!("unknown mutator: {name}"),
    }
}

#[test_case("input")]
#[test_case("operation")]
#[test_case("splice")]
#[test_case("combine")]
#[test_case("codegen")]
#[test_case("gc_injection")]
#[test_case("chain_depth")]
#[test_case("table_size")]
#[test_case("interleave")]
#[test_case("loadstring_wrap")]
#[test_case("metamethod_swap")]
#[test_case("callback_gc")]
fn mutator_produces_valid_ir(name: &str) {
    let (mutator, programs) = mutator_by_name(name);
    assert_mutator_valid(mutator.as_ref(), &programs, 20);
}

#[test]
fn combine_bumps_next_var() {
    let mut rng = StdRng::seed_from_u64(42);
    let mut host = make_three_var_program();
    let orig_next_var = host.next_var;

    assert!(CombineMutator.mutate(&mut host, &mut rng));
    assert!(host.next_var > orig_next_var);
    assert!(host.validate().is_ok());
}

#[test]
fn interleave_preserves_block_structure() {
    let p = make_block_program();
    let mut rng = StdRng::seed_from_u64(42);

    for _ in 0..50 {
        let mut candidate = p.clone();
        if InterleaveMutator.mutate(&mut candidate, &mut rng) {
            assert!(
                candidate.validate().is_ok(),
                "InterleaveMutator broke block structure: {:?}",
                candidate.validate().unwrap_err()
            );
        }
    }
}

#[test]
fn engine_reverts_on_invalid() {
    struct BreakingMutator;
    impl Mutator for BreakingMutator {
        fn name(&self) -> &'static str {
            "BreakingMutator"
        }
        fn mutate(&self, program: &mut Program, _rng: &mut dyn rand::RngCore) -> bool {
            program.instructions.push(fuzzilua_ir::Instruction {
                op: Op::EndIf,
                inputs: vec![],
                outputs: vec![],
            });
            true
        }
    }

    let mut p = make_three_var_program();
    let original_len = p.instructions.len();
    let mut rng = StdRng::seed_from_u64(42);

    let engine =
        MutationEngine::from_mutators(vec![(Box::new(BreakingMutator) as Box<dyn Mutator>, 1.0)]);
    let history = engine.mutate(&mut p, &mut rng);

    assert!(history.is_empty());
    assert_eq!(p.instructions.len(), original_len);
    assert!(p.validate().is_ok());
}

#[test]
fn fuzz_all_mutators() {
    let generators = all_generators();
    let engine = MutationEngine::new();
    let mut rng = StdRng::seed_from_u64(FUZZ_SEED);

    let mut total_mutations = 0u64;
    let mut per_mutator_applied: std::collections::HashMap<&str, u64> =
        std::collections::HashMap::new();

    for i in 0..FUZZ_PROGRAMS {
        let budget = [10, 30, 50, 80][i % 4];
        let mut program = generate_program(&mut rng, budget, 5, &generators);
        assert!(
            program.validate().is_ok(),
            "seed program {i} invalid before mutation"
        );

        for round in 0..MUTATIONS_PER_PROGRAM {
            if program.instructions.len() > MAX_PROGRAM_SIZE {
                program.instructions.truncate(MAX_PROGRAM_SIZE);
                let mut depth = 0i32;
                let mut safe_len = 0;
                for (i, instr) in program.instructions.iter().enumerate() {
                    if instr.op.opens_block().is_some() {
                        depth += 1;
                    }
                    if instr.op.closes_block().is_some() {
                        depth -= 1;
                    }
                    if depth == 0 {
                        safe_len = i + 1;
                    }
                }
                program.instructions.truncate(safe_len);
            }

            let history = engine.mutate(&mut program, &mut rng);
            total_mutations += history.len() as u64;
            for name in &history {
                *per_mutator_applied.entry(name).or_insert(0) += 1;
            }

            if round % 10 == 0 {
                assert!(
                    program.validate().is_ok(),
                    "program {i} round {round} invalid after mutations {history:?}: {:?}",
                    program.validate().unwrap_err()
                );
            }
        }
    }

    eprintln!(
        "fuzz_all_mutators: {FUZZ_PROGRAMS}x{MUTATIONS_PER_PROGRAM} rounds, {total_mutations} mutations applied"
    );
    for (name, count) in &per_mutator_applied {
        eprintln!("  {name}: {count} applied");
    }
}

#[test]
fn hybrid_engine_produces_valid_programs() {
    let engine = HybridEngine::new();
    let mut rng = StdRng::seed_from_u64(FUZZ_SEED);

    for _ in 0..100 {
        let program = engine.generate(&mut rng);
        assert!(
            program.validate().is_ok(),
            "HybridEngine produced invalid IR: {:?}",
            program.validate().unwrap_err()
        );
        assert!(!program.instructions.is_empty());
    }
}
