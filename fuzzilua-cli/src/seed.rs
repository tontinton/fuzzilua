use fuzzilua_corpus::Corpus;
use fuzzilua_gen::{all_generators, generate_program};
use fuzzilua_ir::lift;
use fuzzilua_target::{ExecStatus, Target};
use rand::RngCore;
use tracing::info;

const SEED_COUNT: usize = 200;
const BLIND_SEED_COUNT: usize = 100;
const MIN_BUDGET: usize = 5;
const MAX_BUDGET: usize = 40;
const MAX_DEPTH: usize = 4;

pub fn seed_corpus(target: &mut dyn Target, rng: &mut dyn RngCore, corpus: &mut Corpus) -> u64 {
    let generators = all_generators();
    let mut kept = 0u64;
    let mut successful = 0usize;

    for i in 0..SEED_COUNT {
        let budget = MIN_BUDGET + (i % (MAX_BUDGET - MIN_BUDGET));
        let program = generate_program(rng, budget, MAX_DEPTH, &generators);
        let script = lift(&program);

        let result = match target.execute(&script) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(iter = i, "seed execution error: {e}");
                let _ = target.restart();
                continue;
            }
        };

        match result.status {
            ExecStatus::Ok | ExecStatus::RuntimeError(_) => {
                let mut coverage = target.collect_coverage();
                coverage.classify_counts();

                if corpus.add(program.clone(), coverage) {
                    kept += 1;
                } else {
                    successful += 1;
                }
            }
            ExecStatus::ConnectionLost => {
                let _ = target.restart();
            }
            _ => {}
        }

        let _ = target.reset();

        if (i + 1) % 50 == 0 {
            info!(
                "seeded {}/{SEED_COUNT} programs, {kept} with new coverage",
                i + 1
            );
        }
    }

    if kept == 0 && successful > 0 {
        let count = BLIND_SEED_COUNT.min(successful);
        info!("no coverage-guided seeds found, blind-seeding {count} programs");
        for _ in 0..count {
            let budget = MIN_BUDGET + (rng.next_u32() as usize % (MAX_BUDGET - MIN_BUDGET));
            let program = generate_program(rng, budget, MAX_DEPTH, &generators);
            corpus.add_blind(program);
            kept += 1;
        }
    }

    info!("seeding complete: {kept}/{SEED_COUNT} programs kept");
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuzzilua_corpus::WeightedScheduler;
    use fuzzilua_coverage::CoverageBitmap;
    use fuzzilua_target::MockTarget;

    #[test]
    fn seed_corpus_with_mock_produces_entries() {
        let responses = vec![fuzzilua_target::ExecStatus::Ok; SEED_COUNT];
        let mut target = MockTarget::new(responses, 64, 64);

        let mut bm = CoverageBitmap::new(64, 64);
        bm.edge_bytes_mut()[0] = 1;
        target.set_coverage(bm);

        let tmp = tempfile::tempdir().unwrap();
        let mut corpus = Corpus::new(Box::new(WeightedScheduler), tmp.path(), 64, 64);

        let mut rng = rand::rng();
        let kept = seed_corpus(&mut target, &mut rng, &mut corpus);

        assert!(kept >= 1, "should keep at least one seed, got {kept}");
        assert_eq!(corpus.len(), kept as usize);
    }

    #[test]
    fn seed_corpus_blind_seeds_when_no_coverage() {
        let responses = vec![fuzzilua_target::ExecStatus::Ok; SEED_COUNT];
        let mut target = MockTarget::new(responses, 64, 64);

        let tmp = tempfile::tempdir().unwrap();
        let mut corpus = Corpus::new(Box::new(WeightedScheduler), tmp.path(), 64, 64);

        let mut rng = rand::rng();
        let kept = seed_corpus(&mut target, &mut rng, &mut corpus);

        assert!(kept > 0, "blind seeding should keep some programs");
        assert!(
            kept <= BLIND_SEED_COUNT as u64,
            "should not exceed blind seed limit"
        );
    }
}
