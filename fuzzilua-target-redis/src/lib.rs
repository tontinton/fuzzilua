//! Redis Lua 5.1 fuzzer target.
//!
//! Minimal RESP client (no redis crate dep): single `BufReader<TcpStream>`, uses
//! `get_ref()`/`get_mut()` for writes and timeout-setting (no fd duplication).
//! The RESP parser is generic over `BufRead + Read` so the same code handles
//! TcpStream in prod and Cursor in tests.
//!
//! Process lifecycle: SIGTERM -> 1s grace -> SIGKILL, exit status stored at every
//! reap point. Stderr is drained by a background thread capped at 10K lines to
//! prevent unbounded memory growth. ASan/UBSan report extraction happens on the
//! drained stderr lines.
//!
//! SharedCoverage is a plain field (not Option): always created in `spawn()`,
//! never None. Timeouts use `TcpStream::set_read_timeout` per execution;
//! consecutive timeouts past a threshold trigger a full restart.
//!
//! `pick_random_port()` has a TOCTOU race (port may be reused between our bind
//! and Redis's bind). Acceptable for fuzzer workers; the caller retries on spawn failure.
//!
//! Only `RedisConfig` and `RedisTarget` are pub. RESP internals are crate-private.

mod resp;
mod target;

pub use target::{RedisConfig, RedisTarget};
