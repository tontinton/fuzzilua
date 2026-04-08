use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn fuzzilua_bin() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_fuzzilua"));
    assert!(path.exists(), "binary not found at {}", path.display());
    path
}

fn redis_bin() -> Option<PathBuf> {
    std::env::var("FUZZILUA_REDIS_BIN").ok().map(PathBuf::from)
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
    assert!(stdout.contains("--corpus"));
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use fuzzilua_corpus::{Corpus, WeightedScheduler};
    use fuzzilua_coverage::CoverageBitmap;
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
        ubsan_report: None,
        script: String::new(),
    };

    let mut target = MockTarget::new(vec![ExecStatus::Crash(crash_info); 5], 64, 64);
    let mut cov2 = CoverageBitmap::new(64, 64);
    cov2.edge_bytes_mut()[1] = 1;
    target.set_coverage(cov2);

    let config = fuzzilua_cli::FuzzerConfig {
        max_iters: Some(5),
        crash_dir: crash_dir.clone(),
    };

    let stats = fuzzilua_cli::run_fuzzer_loop(
        &mut target,
        &mut corpus,
        &MutationEngine::new(),
        &config,
        Arc::new(AtomicBool::new(false)),
        &mut rng,
    );

    assert_eq!(stats.crashes(), 5);
    assert_eq!(stats.total_execs(), 5);

    for ext in ["lua", "bin", "txt"] {
        assert!(
            count_files_with_ext(&crash_dir, ext) > 0,
            "should have .{ext} crash files"
        );
    }
}
