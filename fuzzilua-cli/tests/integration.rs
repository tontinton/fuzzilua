use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn fuzzilua_bin() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fuzzilua"));
    assert!(path.exists(), "binary not found at {}", path.display());
    path
}

fn redis_bin() -> Option<PathBuf> {
    std::env::var(fuzzilua_target_redis::ENV_REDIS_BIN)
        .ok()
        .map(PathBuf::from)
}

fn count_files_with_ext(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .count()
}

#[test]
fn binary_runs_help() {
    let output = Command::new(fuzzilua_bin())
        .arg("--help")
        .output()
        .expect("failed to run fuzzilua --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fuzzilua"));
}

#[test]
fn smoke_test_500_iterations() {
    let Some(redis) = redis_bin() else {
        eprintln!("skipping: FUZZILUA_REDIS_BIN not set");
        return;
    };

    let tmp = tempfile::tempdir().expect("tmpdir");
    let corpus_dir = tmp.path().join("corpus");

    let output = Command::new(fuzzilua_bin())
        .args([
            "--redis-bin",
            redis.to_str().unwrap(),
            "--corpus",
            corpus_dir.to_str().unwrap(),
            "--max-iters",
            "500",
            "--jobs",
            "1",
            "--edge-size",
            "4096",
            "--gc-size",
            "4096",
            "--timeout",
            "5s",
        ])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run fuzzer");

    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- fuzzer stderr ---\n{stderr}");

    assert!(
        output.status.success(),
        "fuzzer exited with {:?}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        count_files_with_ext(&corpus_dir, "bin") > 0,
        "corpus should have .bin files on disk"
    );
    assert!(stderr.contains("fuzzing complete"));
}

#[test]
fn multi_worker_500_iterations() {
    let Some(redis) = redis_bin() else {
        eprintln!("skipping: FUZZILUA_REDIS_BIN not set");
        return;
    };

    let tmp = tempfile::tempdir().expect("tmpdir");
    let corpus_dir = tmp.path().join("corpus");
    let stats_json = tmp.path().join("stats.jsonl");

    let output = Command::new(fuzzilua_bin())
        .args([
            "--redis-bin",
            redis.to_str().unwrap(),
            "--corpus",
            corpus_dir.to_str().unwrap(),
            "--max-iters",
            "500",
            "--jobs",
            "4",
            "--edge-size",
            "4096",
            "--gc-size",
            "4096",
            "--timeout",
            "5s",
            "--stats-interval",
            "1s",
            "--stats-json",
            stats_json.to_str().unwrap(),
        ])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run fuzzer");

    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- fuzzer stderr ---\n{stderr}");

    assert!(
        output.status.success(),
        "fuzzer exited with {:?}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        count_files_with_ext(&corpus_dir, "bin") > 0,
        "corpus should have .bin files on disk"
    );
    assert!(stderr.contains("fuzzing complete"));
}

#[test]
fn graceful_shutdown_on_sigint() {
    let Some(redis) = redis_bin() else {
        eprintln!("skipping: FUZZILUA_REDIS_BIN not set");
        return;
    };

    let tmp = tempfile::tempdir().expect("tmpdir");
    let corpus_dir = tmp.path().join("corpus");

    let mut child = Command::new(fuzzilua_bin())
        .args([
            "--redis-bin",
            redis.to_str().unwrap(),
            "--corpus",
            corpus_dir.to_str().unwrap(),
            "--max-iters",
            "1000000",
            "--jobs",
            "1",
            "--edge-size",
            "4096",
            "--gc-size",
            "4096",
            "--timeout",
            "5s",
        ])
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn fuzzer");

    let pid = nix::unistd::Pid::from_raw(child.id() as i32);

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            panic!("timed out waiting for corpus files to appear");
        }
        if corpus_dir.exists()
            && std::fs::read_dir(&corpus_dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT).expect("failed to send SIGINT");

    let output = child.wait_with_output().expect("failed to wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- fuzzer stderr ---\n{stderr}");

    assert!(
        output.status.success(),
        "fuzzer should exit cleanly after SIGINT, got {:?}",
        output.status.code()
    );
    assert!(stderr.contains("fuzzing complete"));
}

#[test]
fn crash_detection_with_mock_target() {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, RwLock};

    use fuzzilua_cli::{AtomicStats, CrashDb, SharedState, WorkerConfig, run_worker_loop};
    use fuzzilua_corpus::{Corpus, WeightedScheduler};
    use fuzzilua_coverage::{AtomicBitmap, CoverageBitmap};
    use fuzzilua_gen::{all_generators, generate_program};
    use fuzzilua_mutate::MutationEngine;
    use fuzzilua_target::{CrashInfo, ExecStatus, MockTarget};

    let tmp = tempfile::tempdir().expect("tmpdir");
    let corpus_dir = tmp.path().join("corpus");
    let crash_dir = corpus_dir.join("crashes");
    std::fs::create_dir_all(&crash_dir).unwrap();

    let mut corpus = Corpus::new(Box::new(WeightedScheduler), &corpus_dir, 64, 64);
    let mut rng = rand::rng();
    let prog = generate_program(&mut rng, 10, 3, &all_generators());
    let mut cov = CoverageBitmap::new(64, 64);
    cov.edge_bytes_mut()[0] = 1;
    corpus.add(prog, cov);

    let crash_info = CrashInfo {
        signal: Some(11),
        asan_report: Some("ERROR: AddressSanitizer: heap-use-after-free\nSUMMARY: boom".into()),
        script: String::new(),
    };

    let mut target = MockTarget::new(vec![ExecStatus::Crash(crash_info); 5], 64, 64);
    let mut cov2 = CoverageBitmap::new(64, 64);
    cov2.edge_bytes_mut()[1] = 1;
    target.set_coverage(cov2);

    let shared = SharedState {
        corpus: RwLock::new(corpus),
        coverage: AtomicBitmap::new(64, 64),
        crash_db: Mutex::new(CrashDb::new()),
        stats: AtomicStats::new(),
        shutdown: Arc::new(AtomicBool::new(false)),
        generation_ratio: fuzzilua_cli::AtomicF64::new(0.0),
    };

    run_worker_loop(
        &mut target,
        &shared,
        &MutationEngine::new(),
        &WorkerConfig {
            max_iters: Some(5),
            crash_dir: crash_dir.clone(),
            minimize: false,
            worker_id: 0,
        },
        &mut rng,
    );

    assert_eq!(shared.stats.crashes(), 5);
    assert_eq!(shared.stats.total_execs(), 5);
    assert!(count_files_with_ext(&crash_dir, "bin") > 0);
    assert_eq!(shared.crash_db.lock().unwrap().unique_count(), 1);
}

#[test]
fn lock_contention_corpus_correctness() {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, RwLock};
    use std::thread;

    use fuzzilua_cli::{AtomicStats, CrashDb, SharedState};
    use fuzzilua_corpus::{Corpus, WeightedScheduler};
    use fuzzilua_coverage::{AtomicBitmap, CoverageBitmap};
    use fuzzilua_gen::{all_generators, generate_program};

    let tmp = tempfile::tempdir().expect("tmpdir");
    let corpus = Corpus::new(Box::new(WeightedScheduler), tmp.path(), 64, 64);

    let shared = Arc::new(SharedState {
        corpus: RwLock::new(corpus),
        coverage: AtomicBitmap::new(64, 64),
        crash_db: Mutex::new(CrashDb::new()),
        stats: AtomicStats::new(),
        shutdown: Arc::new(AtomicBool::new(false)),
        generation_ratio: fuzzilua_cli::AtomicF64::new(0.3),
    });

    {
        let mut rng = rand::rng();
        let prog = generate_program(&mut rng, 10, 3, &all_generators());
        let mut cov = CoverageBitmap::new(64, 64);
        cov.edge_bytes_mut()[0] = 1;
        shared.coverage.merge(&cov);
        shared.corpus.write().unwrap().add(prog, cov);
    }

    const THREADS: usize = 8;
    const ITERS: usize = 10_000;

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let mut rng = rand::rng();
                let generators = all_generators();
                for i in 0..ITERS {
                    if i % 3 == 0 {
                        let prog = generate_program(&mut rng, 5, 2, &generators);
                        let mut cov = CoverageBitmap::new(64, 64);
                        cov.edge_bytes_mut()[(t * ITERS + i) % 64] = 1;
                        if shared.coverage.merge_if_new(&cov) {
                            shared.corpus.write().unwrap().add_unchecked(prog, cov);
                        }
                    } else {
                        let corpus = shared.corpus.read().unwrap();
                        if !corpus.is_empty() {
                            let _entry = corpus.select(&mut rng);
                        }
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    assert!(
        !shared.corpus.read().unwrap().is_empty(),
        "corpus should have at least the seed entry"
    );
}
