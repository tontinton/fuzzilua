//! Redis Lua 5.1 fuzzer target.
//!
//! Minimal RESP client over `BufReader<TcpStream>`, no redis crate dep. The RESP
//! parser is generic over `BufRead + Read` for testability with `Cursor`.
//!
//! Process lifecycle: SIGTERM, 1s grace, then SIGKILL. Stderr is drained by a
//! background thread (capped at 10K lines) for ASan/UBSan report extraction.
//!
//! `SharedCoverage` is always present (created at `spawn()`). Consecutive timeouts
//! past a threshold trigger a full process restart.

mod resp;
mod target;

pub use target::{RedisConfig, RedisTarget};
