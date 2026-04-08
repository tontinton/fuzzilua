use fuzzilua_gen::{all_generators, generate_program};
use fuzzilua_ir::{Op, Program, Variable, lift};
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::engine::MutationEngine;
use crate::mutators::{
    CodeGenMutator, CombineMutator, GcInjectionMutator, InputMutator, Mutator, OperationMutator,
    SpliceMutator,
};

const FUZZ_SEED: u64 = 0xCAFE_1234_5678;
const FUZZ_PROGRAMS: usize = 100;
const MUTATIONS_PER_PROGRAM: usize = 20;

fn make_three_var_program() -> Program {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.emit(Op::LoadString("hello".into()), vec![], vec![Variable(1)]);
    p.emit(Op::CreateTable, vec![], vec![Variable(2)]);
    p.emit(
        Op::BinaryOp(fuzzilua_ir::BinOp::Add),
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

#[test]
fn input_mutator_produces_valid_ir() {
    assert_mutator_valid(&InputMutator, &[make_three_var_program()], 20);
}

#[test]
fn operation_mutator_produces_valid_ir() {
    assert_mutator_valid(
        &OperationMutator,
        &[make_three_var_program(), make_metamethod_program()],
        20,
    );
}

#[test]
fn splice_mutator_produces_valid_ir() {
    assert_mutator_valid(
        &SpliceMutator,
        &[make_three_var_program(), make_block_program()],
        20,
    );
}

#[test]
fn combine_mutator_produces_valid_ir() {
    let mut rng = StdRng::seed_from_u64(42);
    let mutator = CombineMutator;

    let mut host = make_three_var_program();
    let orig_next_var = host.next_var;

    assert!(mutator.mutate(&mut host, &mut rng));
    assert!(host.next_var > orig_next_var, "next_var not bumped");
    assert!(host.validate().is_ok());
}

#[test]
fn code_gen_mutator_produces_valid_ir() {
    assert_mutator_valid(&CodeGenMutator, &[make_three_var_program()], 20);
}

#[test]
fn gc_injection_produces_valid_ir() {
    assert_mutator_valid(
        &GcInjectionMutator,
        &[make_three_var_program(), make_block_program()],
        20,
    );
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

    assert!(
        history.is_empty(),
        "breaking mutations should all be reverted"
    );
    assert_eq!(p.instructions.len(), original_len);
    assert!(p.validate().is_ok());
}

#[test]
fn fuzz_mutation_engine() {
    let generators = all_generators();
    let engine = MutationEngine::new();
    let mut rng = StdRng::seed_from_u64(FUZZ_SEED);

    let mut total_mutations = 0u64;

    for i in 0..FUZZ_PROGRAMS {
        let budget = [10, 50, 200, 500][i % 4];
        let mut program = generate_program(&mut rng, budget, 5, &generators);
        assert!(
            program.validate().is_ok(),
            "seed program {i} invalid before mutation"
        );

        for round in 0..MUTATIONS_PER_PROGRAM {
            let history = engine.mutate(&mut program, &mut rng);
            total_mutations += history.len() as u64;

            assert!(
                program.validate().is_ok(),
                "program {i} round {round} invalid after mutations {history:?}: {:?}",
                program.validate().unwrap_err()
            );

            let lua = lift(&program);
            assert!(
                !lua.is_empty() || program.instructions.iter().all(|i| matches!(i.op, Op::Nop)),
                "program {i} round {round}: lifted to empty Lua"
            );
        }
    }

    eprintln!(
        "fuzz_mutation_engine: {FUZZ_PROGRAMS}x{MUTATIONS_PER_PROGRAM} rounds, {total_mutations} mutations applied"
    );
}
