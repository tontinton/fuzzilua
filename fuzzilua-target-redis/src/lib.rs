//! Redis Lua 5.1 fuzzer target.
//!
//! Owns the Redis child process lifecycle and a minimal RESP client (no redis
//! crate dep). Coverage is read from shared memory; sanitizer reports are
//! extracted from stderr.
//!
//! All `FUZZILUA_*` environment variables are defined as `ENV_*` constants below.

mod resp;
mod target;

/// Env var: shared-memory region name for edge coverage bitmap.
pub const ENV_SHM_EDGE: &str = "FUZZILUA_SHM_EDGE";

/// Env var: shared-memory region name for GC-phase coverage bitmap.
pub const ENV_SHM_GC: &str = "FUZZILUA_SHM_GC";

/// Env var: allocation failure probability (0.0-1.0) for alloc-fail injection mode.
pub const ENV_ALLOC_FAIL_PROB: &str = "FUZZILUA_ALLOC_FAIL_PROB";

/// Env var (test-only): path to the instrumented `redis-server` binary.
pub const ENV_REDIS_BIN: &str = "FUZZILUA_REDIS_BIN";

pub use target::{RedisConfig, RedisTarget};
