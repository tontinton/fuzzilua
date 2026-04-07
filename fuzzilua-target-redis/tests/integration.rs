use std::env;
use std::path::PathBuf;
use std::time::Duration;

use fuzzilua_target::{ExecStatus, Target};
use fuzzilua_target_redis::{RedisConfig, RedisTarget};

fn spawn_redis() -> Option<RedisTarget> {
    let binary = env::var("FUZZILUA_REDIS_BIN").ok().map(PathBuf::from)?;
    let config = RedisConfig {
        binary,
        exec_timeout: Duration::from_secs(2),
        consecutive_timeout_threshold: 3,
        ..RedisConfig::default()
    }
    .with_random_port();
    Some(RedisTarget::spawn(config).expect("failed to spawn redis"))
}

#[test]
fn eval_return_integer() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    let result = target.execute("return 1").expect("execute failed");
    assert!(
        matches!(result.status, ExecStatus::Ok),
        "{:?}",
        result.status
    );
}

#[test]
fn eval_runtime_error() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    let result = target.execute("error('boom')").expect("execute failed");
    assert!(
        matches!(result.status, ExecStatus::RuntimeError(ref msg) if msg.contains("boom")),
        "{:?}",
        result.status
    );
}

#[test]
fn eval_timeout() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    let result = target.execute("while true do end").expect("execute failed");
    assert!(
        matches!(result.status, ExecStatus::Timeout),
        "{:?}",
        result.status
    );
}

#[test]
fn coverage_nonzero_after_eval() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    target.execute("return 1").expect("execute failed");
    let (edge_bits, _) = target.collect_coverage().count_bits();
    assert!(edge_bits > 0, "expected nonzero edge coverage after EVAL");
}

#[test]
fn reset_clears_bitmaps() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    target.execute("return 1").expect("execute failed");
    target.reset().expect("reset failed");
    let (edge_bits, gc_bits) = target.collect_coverage().count_bits();
    assert_eq!((edge_bits, gc_bits), (0, 0));
}

#[test]
fn restart_recovers() {
    let Some(mut target) = spawn_redis() else {
        return;
    };
    target.execute("return 1").expect("initial execute failed");
    assert!(target.is_alive());

    target.restart().expect("restart failed");
    assert!(target.is_alive());

    let result = target
        .execute("return 42")
        .expect("execute after restart failed");
    assert!(
        matches!(result.status, ExecStatus::Ok),
        "{:?}",
        result.status
    );
}
