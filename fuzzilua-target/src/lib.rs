//! Target trait and execution types for fuzzer backends.
//!
//! `ExecStatus` holds semantic outcomes (crash, timeout, runtime error).
//! `TargetError` holds infrastructure failures (spawn, connection, protocol).
//! A timeout is a normal execution outcome, not an error.
//!
//! The `Target` trait is object-safe and sync-only.

mod mock;

pub use mock::MockTarget;

use std::time::Duration;

use fuzzilua_coverage::CoverageBitmap;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Execution {
    pub status: ExecStatus,
    pub stderr: String,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub enum ExecStatus {
    Ok,
    RuntimeError(String),
    Crash(CrashInfo),
    Timeout,
    ConnectionLost,
}

#[derive(Debug, Clone)]
pub struct CrashInfo {
    pub signal: Option<i32>,
    pub asan_report: Option<String>,
    pub script: String,
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub available_globals: Vec<String>,
    pub available_modules: Vec<String>,
    pub blocked_functions: Vec<String>,
    pub max_execution_time: Duration,
    pub has_coroutines: bool,
    pub has_loadstring: bool,
}

impl SandboxConfig {
    /// Redis Lua 5.1 sandbox as documented in redis/src/scripting.c.
    pub fn redis_lua51(exec_timeout: Duration) -> Self {
        Self {
            available_globals: vec![
                "redis".into(),
                "KEYS".into(),
                "ARGV".into(),
                "string".into(),
                "table".into(),
                "math".into(),
                "struct".into(),
                "cjson".into(),
                "cmsgpack".into(),
                "bit".into(),
                "tonumber".into(),
                "tostring".into(),
                "type".into(),
                "next".into(),
                "pairs".into(),
                "ipairs".into(),
                "select".into(),
                "unpack".into(),
                "rawget".into(),
                "rawset".into(),
                "rawequal".into(),
                "rawlen".into(),
                "pcall".into(),
                "xpcall".into(),
                "error".into(),
                "assert".into(),
                "collectgarbage".into(),
                "setmetatable".into(),
                "getmetatable".into(),
            ],
            available_modules: vec![
                "string".into(),
                "table".into(),
                "math".into(),
                "struct".into(),
                "cjson".into(),
                "cmsgpack".into(),
                "bit".into(),
            ],
            blocked_functions: vec![
                "os".into(),
                "io".into(),
                "debug".into(),
                "loadfile".into(),
                "dofile".into(),
                "package".into(),
                "require".into(),
            ],
            max_execution_time: exec_timeout,
            has_coroutines: true,
            has_loadstring: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum TargetError {
    #[error("process spawn failed: {0}")]
    SpawnFailed(#[source] std::io::Error),
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("protocol error: {0}")]
    ProtocolError(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub trait Target {
    fn execute(&mut self, script: &str) -> Result<Execution, TargetError>;
    fn reset(&mut self) -> Result<(), TargetError>;
    fn restart(&mut self) -> Result<(), TargetError>;
    fn collect_coverage(&self) -> CoverageBitmap;
    fn is_alive(&mut self) -> bool;
    fn sandbox(&self) -> &SandboxConfig;
}
