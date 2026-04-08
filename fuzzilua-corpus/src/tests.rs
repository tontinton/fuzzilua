use fuzzilua_coverage::CoverageBitmap;
use fuzzilua_ir::{BinOp, GcMode, Op, Program, Variable};
use fuzzilua_target::{ExecStatus, MockTarget, SandboxConfig, Target, TargetError};
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::{
    Corpus, CorpusEntry, CorpusScheduler, FocusedScheduler, UniformScheduler, WeightedScheduler,
    minimize,
};

const EDGE_SIZE: usize = 64;
const GC_SIZE: usize = 32;

fn make_coverage(edge_bits: &[usize], gc_bits: &[usize]) -> CoverageBitmap {
    let mut bm = CoverageBitmap::new(EDGE_SIZE, GC_SIZE);
    for &i in edge_bits {
        bm.edge_bytes_mut()[i] = 1;
    }
    for &i in gc_bits {
        bm.gc_bytes_mut()[i] = 1;
    }
    bm
}

fn make_corpus() -> Corpus {
    Corpus::new(
        Box::new(UniformScheduler),
        std::env::temp_dir().join(format!("fuzzilua_test_corpus_{}", std::process::id())),
        EDGE_SIZE,
        GC_SIZE,
    )
}

fn make_corpus_in(dir: &std::path::Path) -> Corpus {
    Corpus::new(Box::new(UniformScheduler), dir, EDGE_SIZE, GC_SIZE)
}

#[test]
fn add_and_coverage_tracking() {
    let mut corpus = make_corpus();
    assert_eq!(corpus.total_coverage(), (0, 0));

    assert!(corpus.add(Program::new(), make_coverage(&[0, 1, 2], &[])));
    assert_eq!(corpus.len(), 1);
    assert_eq!(corpus.total_coverage(), (3, 0));

    assert!(!corpus.add(Program::new(), make_coverage(&[0, 1], &[])));
    assert_eq!(corpus.len(), 1);

    assert!(!corpus.add(Program::new(), make_coverage(&[0, 1, 2], &[])));
    assert_eq!(corpus.len(), 1);

    assert!(corpus.add(Program::new(), make_coverage(&[3], &[])));
    assert_eq!(corpus.len(), 2);
    assert_eq!(corpus.total_coverage(), (4, 0));

    assert!(corpus.add(Program::new(), make_coverage(&[], &[0])));
    assert_eq!(corpus.len(), 3);
    assert_eq!(corpus.total_coverage(), (4, 1));

    let mut rng = SmallRng::seed_from_u64(42);
    let entry = corpus.select(&mut rng);
    assert!(entry.coverage.total_nonzero() > 0);
}

#[test]
#[should_panic(expected = "cannot select from empty corpus")]
fn select_panics_on_empty() {
    let corpus = make_corpus();
    let mut rng = SmallRng::seed_from_u64(42);
    corpus.select(&mut rng);
}

// --- Scheduler tests ---

#[test]
fn uniform_scheduler_hits_all_entries() {
    let entries: Vec<CorpusEntry> = (0..10)
        .map(|i| {
            let coverage = make_coverage(&[i], &[]);
            CorpusEntry {
                cached_nonzero: coverage.total_nonzero(),
                program: Program::new(),
                coverage,
                mutation_count: 0,
            }
        })
        .collect();

    let scheduler = UniformScheduler;
    let mut rng = SmallRng::seed_from_u64(12345);
    let mut seen = [false; 10];

    for _ in 0..10_000 {
        let idx = scheduler.select(&entries, &mut rng);
        assert!(idx < entries.len());
        seen[idx] = true;
    }

    assert!(
        seen.iter().all(|&s| s),
        "all entries should be selected at least once"
    );
}

#[test]
fn weighted_scheduler_favors_more_coverage() {
    let cov1 = make_coverage(&[0], &[]);
    let cov2 = make_coverage(&(0..50).collect::<Vec<_>>(), &[]);
    let entries = vec![
        CorpusEntry {
            cached_nonzero: cov1.total_nonzero(),
            program: Program::new(),
            coverage: cov1,
            mutation_count: 0,
        },
        CorpusEntry {
            cached_nonzero: cov2.total_nonzero(),
            program: Program::new(),
            coverage: cov2,
            mutation_count: 0,
        },
    ];

    let scheduler = WeightedScheduler;
    let mut rng = SmallRng::seed_from_u64(99);
    let mut counts = [0u32; 2];

    for _ in 0..10_000 {
        counts[scheduler.select(&entries, &mut rng)] += 1;
    }

    assert!(
        counts[1] > counts[0],
        "entry with 50 bits ({}) should be picked more than entry with 1 bit ({})",
        counts[1],
        counts[0]
    );
}

#[test]
fn weighted_scheduler_penalizes_high_mutation_count() {
    let cov1 = make_coverage(&(0..50).collect::<Vec<_>>(), &[]);
    let cov2 = make_coverage(&(0..50).collect::<Vec<_>>(), &[]);
    let entries = vec![
        CorpusEntry {
            cached_nonzero: cov1.total_nonzero(),
            program: Program::new(),
            coverage: cov1,
            mutation_count: 0,
        },
        CorpusEntry {
            cached_nonzero: cov2.total_nonzero(),
            program: Program::new(),
            coverage: cov2,
            mutation_count: 100,
        },
    ];

    let scheduler = WeightedScheduler;
    let mut rng = SmallRng::seed_from_u64(77);
    let mut counts = [0u32; 2];

    for _ in 0..10_000 {
        counts[scheduler.select(&entries, &mut rng)] += 1;
    }

    assert!(
        counts[0] > counts[1],
        "low mutation_count ({}) should be picked more than high mutation_count ({})",
        counts[0],
        counts[1]
    );
}

#[test]
fn focused_scheduler_favors_gc_coverage() {
    let cov1 = make_coverage(&[], &[0]);
    let cov2 = make_coverage(&[], &(0..30).collect::<Vec<_>>());
    let entries = vec![
        CorpusEntry {
            cached_nonzero: cov1.total_nonzero(),
            program: Program::new(),
            coverage: cov1,
            mutation_count: 0,
        },
        CorpusEntry {
            cached_nonzero: cov2.total_nonzero(),
            program: Program::new(),
            coverage: cov2,
            mutation_count: 0,
        },
    ];

    let scheduler = FocusedScheduler;
    let mut rng = SmallRng::seed_from_u64(42);
    let mut counts = [0u32; 2];

    for _ in 0..10_000 {
        counts[scheduler.select(&entries, &mut rng)] += 1;
    }

    assert!(
        counts[1] > counts[0],
        "entry with 30 gc_bits ({}) should be picked more than entry with 1 gc_bit ({})",
        counts[1],
        counts[0]
    );
}

// --- Compaction tests ---

#[test]
fn compact_evicts_strict_subsets() {
    let dir = tempfile::tempdir().unwrap();
    let corpus_dir = dir.path().join("corpus");
    let mut corpus = make_corpus_in(&corpus_dir);

    corpus.add(Program::new(), make_coverage(&[0, 1, 2], &[]));
    corpus.add(Program::new(), make_coverage(&[0, 1, 3], &[]));
    corpus.add(Program::new(), make_coverage(&[4, 5], &[]));
    assert_eq!(corpus.len(), 3);

    corpus.entries[1].coverage = make_coverage(&[0, 1], &[]);
    corpus.compact();

    assert_eq!(corpus.len(), 2, "entry B (subset of A) should be evicted");
    assert_eq!(corpus.entries[0].coverage.count_bits(), (3, 0));
    assert_eq!(corpus.entries[1].coverage.count_bits(), (2, 0));

    let loaded = Corpus::load(Box::new(UniformScheduler), &corpus_dir, EDGE_SIZE, GC_SIZE).unwrap();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn compact_preserves_independent_entries() {
    let mut corpus = make_corpus();

    corpus.compact();
    assert_eq!(corpus.len(), 0);

    corpus.add(Program::new(), make_coverage(&[0], &[]));
    corpus.add(Program::new(), make_coverage(&[1], &[]));
    corpus.add(Program::new(), make_coverage(&[2], &[]));
    corpus.compact();
    assert_eq!(corpus.len(), 3);

    let dir = tempfile::tempdir().unwrap();
    let mut corpus2 = make_corpus_in(dir.path());
    corpus2.add(Program::new(), make_coverage(&[3], &[]));
    corpus2.add(Program::new(), make_coverage(&[], &[3]));
    corpus2.compact();
    assert_eq!(corpus2.len(), 2);
}

// --- Disk persistence tests ---

#[test]
fn save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let corpus_dir = dir.path().join("corpus");

    let mut corpus = make_corpus_in(&corpus_dir);
    for i in 0..5 {
        corpus.add(Program::new(), make_coverage(&[i], &[]));
    }

    let loaded = Corpus::load(Box::new(UniformScheduler), &corpus_dir, EDGE_SIZE, GC_SIZE).unwrap();
    assert_eq!(loaded.len(), 5);
    assert_eq!(corpus.total_coverage(), loaded.total_coverage());

    for entry in std::fs::read_dir(&corpus_dir).unwrap().flatten() {
        let name = entry.file_name();
        assert!(
            !name.to_str().unwrap().starts_with(".tmp_"),
            "temp file should not remain: {name:?}"
        );
    }
}

#[test]
fn crash_save_creates_all_files() {
    let dir = tempfile::tempdir().unwrap();
    let crash_dir = dir.path().join("crashes");

    crate::save_crash(
        &crash_dir,
        &Program::new(),
        11,
        "heap-use-after-free at 0x1234",
    )
    .unwrap();

    let files: Vec<String> = std::fs::read_dir(&crash_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(files.iter().any(|f| f.ends_with(".lua")), "missing .lua");
    assert!(files.iter().any(|f| f.ends_with(".bin")), "missing .bin");
    assert!(files.iter().any(|f| f.ends_with(".txt")), "missing .txt");

    let txt_file = files.iter().find(|f| f.ends_with(".txt")).unwrap();
    let txt_content = std::fs::read_to_string(crash_dir.join(txt_file)).unwrap();
    assert!(txt_content.contains("signal: 11"));
    assert!(txt_content.contains("heap-use-after-free"));
}

#[test]
fn load_skips_corrupt_entries() {
    let dir = tempfile::tempdir().unwrap();
    let corpus_dir = dir.path().join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();

    let mut corpus = make_corpus_in(&corpus_dir);
    corpus.add(Program::new(), make_coverage(&[0], &[]));
    corpus.add(Program::new(), make_coverage(&[1], &[]));

    std::fs::write(corpus_dir.join("entry_corrupt.bin"), b"garbage data").unwrap();

    corpus.add(Program::new(), make_coverage(&[2], &[]));

    let loaded = Corpus::load(Box::new(UniformScheduler), &corpus_dir, EDGE_SIZE, GC_SIZE).unwrap();
    assert_eq!(loaded.len(), 3, "should load 3 valid entries, skip corrupt");
}

#[test]
fn load_nonexistent_dir_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = Corpus::load(
        Box::new(UniformScheduler),
        dir.path().join("nonexistent"),
        EDGE_SIZE,
        GC_SIZE,
    )
    .unwrap();
    assert_eq!(loaded.len(), 0);
}

// --- Minimizer mock ---

struct PersistentCoverageMock {
    inner: MockTarget,
    persistent_coverage: CoverageBitmap,
}

impl PersistentCoverageMock {
    fn new(coverage: CoverageBitmap, exec_count: usize, edge_size: usize, gc_size: usize) -> Self {
        let responses = vec![ExecStatus::Ok; exec_count];
        let mut inner = MockTarget::new(responses, edge_size, gc_size);
        let persistent_coverage = coverage.clone();
        inner.set_coverage(coverage);
        Self {
            inner,
            persistent_coverage,
        }
    }
}

impl Target for PersistentCoverageMock {
    fn execute(&mut self, script: &str) -> Result<fuzzilua_target::Execution, TargetError> {
        self.inner.execute(script)
    }

    fn reset(&mut self) -> Result<(), TargetError> {
        self.inner.reset()?;
        self.inner.set_coverage(self.persistent_coverage.clone());
        Ok(())
    }

    fn restart(&mut self) -> Result<(), TargetError> {
        self.inner.restart()?;
        self.inner.set_coverage(self.persistent_coverage.clone());
        Ok(())
    }

    fn collect_coverage(&self) -> CoverageBitmap {
        self.inner.collect_coverage()
    }

    fn is_alive(&mut self) -> bool {
        self.inner.is_alive()
    }

    fn sandbox(&self) -> &SandboxConfig {
        self.inner.sandbox()
    }
}

fn make_persistent_mock(edge_bits: &[usize]) -> PersistentCoverageMock {
    let mut cov = CoverageBitmap::new(EDGE_SIZE, GC_SIZE);
    for &i in edge_bits {
        cov.edge_bytes_mut()[i] = 1;
    }
    PersistentCoverageMock::new(cov, 200, EDGE_SIZE, GC_SIZE)
}

// --- Minimizer tests ---

#[test]
fn minimize_nop_removal() {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.emit(Op::Nop, vec![], vec![]);
    p.emit(Op::LoadInt(2), vec![], vec![Variable(1)]);
    p.emit(Op::Nop, vec![], vec![]);
    p.emit(Op::Nop, vec![], vec![]);
    p.emit(
        Op::BinaryOp(BinOp::Add),
        vec![Variable(0), Variable(1)],
        vec![Variable(2)],
    );
    p.next_var = 3;

    let mut target = make_persistent_mock(&[0]);
    let result = minimize(&p, &mut target);
    assert!(
        !result.instructions.iter().any(|i| matches!(i.op, Op::Nop)),
        "minimized program should have no Nops"
    );
}

#[test]
fn minimize_gc_consolidation() {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.emit(Op::CollectGarbage(GcMode::Collect), vec![], vec![]);
    p.emit(Op::CollectGarbage(GcMode::Step), vec![], vec![]);
    p.emit(Op::CollectGarbage(GcMode::Collect), vec![], vec![]);
    p.emit(Op::LoadInt(2), vec![], vec![Variable(1)]);
    p.next_var = 2;

    let mut target = make_persistent_mock(&[0]);
    let result = minimize(&p, &mut target);
    let gc_count = result
        .instructions
        .iter()
        .filter(|i| matches!(i.op, Op::CollectGarbage(_)))
        .count();
    assert!(
        gc_count <= 1,
        "should consolidate to at most 1 GC, got {gc_count}"
    );
}

#[test]
fn minimize_dead_variable_elimination() {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.emit(Op::LoadInt(2), vec![], vec![Variable(1)]);
    p.emit(Op::LoadInt(99), vec![], vec![Variable(2)]);
    p.emit(Op::LoadInt(100), vec![], vec![Variable(3)]);
    p.emit(Op::LoadInt(101), vec![], vec![Variable(4)]);
    p.emit(
        Op::BinaryOp(BinOp::Add),
        vec![Variable(0), Variable(1)],
        vec![Variable(5)],
    );
    p.next_var = 6;

    let mut target = make_persistent_mock(&[0]);
    let result = minimize(&p, &mut target);
    let load_count = result
        .instructions
        .iter()
        .filter(|i| matches!(i.op, Op::LoadInt(_)))
        .count();
    assert!(
        load_count <= 3,
        "dead variables should be eliminated, got {load_count}"
    );
}

#[test]
fn minimize_already_minimal_program() {
    let mut p = Program::new();
    p.emit(Op::LoadInt(1), vec![], vec![Variable(0)]);
    p.next_var = 1;

    let mut target = make_persistent_mock(&[0]);
    let result = minimize(&p, &mut target);
    assert_eq!(result.instructions.len(), p.instructions.len());
}

#[test]
fn add_unchecked_bypasses_novelty_gate_and_merges_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let mut corpus = make_corpus_in(dir.path());

    let cov = make_coverage(&[0, 1, 2], &[]);
    corpus.add_unchecked(Program::new(), cov.clone());
    assert_eq!(corpus.len(), 1);

    corpus.add_unchecked(Program::new(), cov);
    assert_eq!(corpus.len(), 2, "add_unchecked should not gate on novelty");

    let (edge_bits, _gc_bits) = corpus.total_coverage();
    assert!(edge_bits >= 3, "global coverage should include merged bits");
}
