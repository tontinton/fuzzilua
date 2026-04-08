use fuzzilua_coverage::CoverageBitmap;
use fuzzilua_ir::Program;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::{Corpus, CorpusEntry, CorpusScheduler, UniformScheduler, WeightedScheduler};

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

    // new edge coverage accepted
    assert!(corpus.add(Program::new(), make_coverage(&[0, 1, 2], &[])));
    assert_eq!(corpus.len(), 1);
    assert_eq!(corpus.total_coverage(), (3, 0));

    // subset rejected
    assert!(!corpus.add(Program::new(), make_coverage(&[0, 1], &[])));
    assert_eq!(corpus.len(), 1);

    // identical rejected
    assert!(!corpus.add(Program::new(), make_coverage(&[0, 1, 2], &[])));
    assert_eq!(corpus.len(), 1);

    // new bits accepted, coverage grows monotonically
    assert!(corpus.add(Program::new(), make_coverage(&[3], &[])));
    assert_eq!(corpus.len(), 2);
    assert_eq!(corpus.total_coverage(), (4, 0));

    // gc-only coverage accepted
    assert!(corpus.add(Program::new(), make_coverage(&[], &[0])));
    assert_eq!(corpus.len(), 3);
    assert_eq!(corpus.total_coverage(), (4, 1));

    // select returns valid entry from populated corpus
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
        .map(|i| CorpusEntry {
            program: Program::new(),
            coverage: make_coverage(&[i], &[]),
            mutation_count: 0,
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
    let entries = vec![
        CorpusEntry {
            program: Program::new(),
            coverage: make_coverage(&[0], &[]),
            mutation_count: 0,
        },
        CorpusEntry {
            program: Program::new(),
            coverage: make_coverage(&(0..50).collect::<Vec<_>>(), &[]),
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
    let entries = vec![
        CorpusEntry {
            program: Program::new(),
            coverage: make_coverage(&(0..50).collect::<Vec<_>>(), &[]),
            mutation_count: 0,
        },
        CorpusEntry {
            program: Program::new(),
            coverage: make_coverage(&(0..50).collect::<Vec<_>>(), &[]),
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

    // make entry B a strict subset of A
    corpus.entries[1].coverage = make_coverage(&[0, 1], &[]);
    corpus.compact();

    assert_eq!(corpus.len(), 2, "entry B (subset of A) should be evicted");
    assert_eq!(corpus.entries[0].coverage.count_bits(), (3, 0));
    assert_eq!(corpus.entries[1].coverage.count_bits(), (2, 0));

    // verify compaction re-persisted
    let loaded = Corpus::load(Box::new(UniformScheduler), &corpus_dir, EDGE_SIZE, GC_SIZE).unwrap();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn compact_preserves_independent_entries() {
    let mut corpus = make_corpus();

    // empty corpus: noop
    corpus.compact();
    assert_eq!(corpus.len(), 0);

    // disjoint entries: all kept
    corpus.add(Program::new(), make_coverage(&[0], &[]));
    corpus.add(Program::new(), make_coverage(&[1], &[]));
    corpus.add(Program::new(), make_coverage(&[2], &[]));
    corpus.compact();
    assert_eq!(corpus.len(), 3);

    // edge vs gc: different namespaces, not subsets of each other
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

    // no temp files left behind
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
