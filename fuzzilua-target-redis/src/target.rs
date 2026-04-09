use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fuzzilua_coverage::{CoverageBitmap, DEFAULT_BITMAP_SIZE, SharedCoverage};
use fuzzilua_target::{CrashInfo, ExecStatus, Execution, SandboxConfig, Target, TargetError};
use tracing::{debug, info, warn};

use crate::resp::{RespClient, RespValue};
use crate::{ENV_SHM_EDGE, ENV_SHM_GC};

const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(1);
const DEFAULT_CONSECUTIVE_TIMEOUT_THRESHOLD: u32 = 3;
const CONNECT_BACKOFF_INITIAL: Duration = Duration::from_millis(10);
const CONNECT_BACKOFF_MAX: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
const MAX_STDERR_LINES: usize = 10_000;

#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub binary: PathBuf,
    pub bind: String,
    pub port: u16,
    pub extra_args: Vec<String>,
    pub extra_env: Vec<(String, String)>,
    pub exec_timeout: Duration,
    pub consecutive_timeout_threshold: u32,
    pub edge_bitmap_size: usize,
    pub gc_bitmap_size: usize,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("redis-server"),
            bind: "127.0.0.1".into(),
            port: 0,
            extra_args: vec![],
            extra_env: vec![],
            exec_timeout: DEFAULT_EXEC_TIMEOUT,
            consecutive_timeout_threshold: DEFAULT_CONSECUTIVE_TIMEOUT_THRESHOLD,
            edge_bitmap_size: DEFAULT_BITMAP_SIZE,
            gc_bitmap_size: DEFAULT_BITMAP_SIZE,
        }
    }
}

impl RedisConfig {
    pub fn with_random_port(mut self) -> Self {
        self.port = pick_random_port();
        self
    }
}

pub struct RedisTarget {
    config: RedisConfig,
    child: Option<Child>,
    exit_status: Option<ExitStatus>,
    client: Option<RespClient>,
    shm: SharedCoverage,
    shm_name: String,
    sandbox_config: SandboxConfig,
    consecutive_timeouts: u32,
    stderr_lines: Arc<Mutex<Vec<String>>>,
}

impl RedisTarget {
    pub fn spawn(config: RedisConfig) -> Result<Self, TargetError> {
        let shm_name = format!("/fuzzilua_{}_{}", std::process::id(), config.port);

        let shm = SharedCoverage::create(&shm_name, config.edge_bitmap_size, config.gc_bitmap_size)
            .map_err(TargetError::SpawnFailed)?;

        let sandbox_config = SandboxConfig::redis_lua51(config.exec_timeout);

        let mut target = Self {
            config,
            child: None,
            exit_status: None,
            client: None,
            shm,
            shm_name,
            sandbox_config,
            consecutive_timeouts: 0,
            stderr_lines: Arc::new(Mutex::new(Vec::new())),
        };

        target.spawn_process()?;
        Ok(target)
    }

    fn spawn_process(&mut self) -> Result<(), TargetError> {
        self.exit_status = None;

        let mut cmd = Command::new(&self.config.binary);
        cmd.arg("--bind")
            .arg(&self.config.bind)
            .arg("--port")
            .arg(self.config.port.to_string())
            .arg("--loglevel")
            .arg("warning")
            .arg("--save")
            .arg("")
            .arg("--appendonly")
            .arg("no");

        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        cmd.env(ENV_SHM_EDGE, &self.shm_name);
        cmd.env(ENV_SHM_GC, &self.shm_name);
        cmd.env(
            "ASAN_OPTIONS",
            "detect_leaks=0:abort_on_error=1:symbolize=1:detect_stack_use_after_return=1:halt_on_error=1",
        );

        for (key, val) in &self.config.extra_env {
            cmd.env(key, val);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(TargetError::SpawnFailed)?;

        let stderr = child.stderr.take().expect("stderr was piped");
        self.start_stderr_drain(stderr);

        self.child = Some(child);

        self.wait_for_ready()?;

        // Discard startup stderr (UBSan noise from module loading, etc.)
        self.drain_stderr();

        let stream = TcpStream::connect((&*self.config.bind, self.config.port))
            .map_err(|e| TargetError::ConnectionFailed(e.to_string()))?;
        let client = RespClient::new(stream);
        self.client = Some(client);

        info!(
            port = self.config.port,
            pid = self.child.as_ref().map(|c| c.id()),
            "redis target spawned"
        );

        Ok(())
    }

    fn start_stderr_drain(&self, stderr: std::process::ChildStderr) {
        let lines = Arc::clone(&self.stderr_lines);

        thread::Builder::new()
            .name("redis-stderr".into())
            .spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            let mut locked = lines.lock().unwrap();
                            if locked.len() < MAX_STDERR_LINES {
                                locked.push(l);
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("failed to spawn stderr drain thread");
    }

    fn drain_stderr(&self) -> String {
        let mut locked = self.stderr_lines.lock().unwrap();
        let lines = std::mem::take(&mut *locked);
        lines.join("\n")
    }

    fn check_sanitizer_report(&self, stderr: &str) -> Option<String> {
        let lines: Vec<&str> = stderr.lines().collect();
        find_asan_report(&lines)
    }

    /// Reap the child if not already reaped. Returns true if still running.
    fn poll_child(&mut self) -> bool {
        if self.exit_status.is_some() {
            return false;
        }
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    self.exit_status = Some(status);
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            },
            None => false,
        }
    }

    /// After a failed send/recv, drain stderr and classify as crash vs connection-lost.
    fn classify_failure(&mut self, script: &str, duration: Duration) -> Execution {
        let stderr = self.drain_stderr();
        let alive = self.poll_child();

        if !alive {
            let signal = self.exit_signal();
            let asan_report = self.check_sanitizer_report(&stderr);

            if signal.is_some() || asan_report.is_some() {
                return Execution {
                    status: ExecStatus::Crash(CrashInfo {
                        signal,
                        asan_report,
                        script: script.to_string(),
                    }),
                    stderr,
                    duration,
                };
            }
        }

        Execution {
            status: ExecStatus::ConnectionLost,
            stderr,
            duration,
        }
    }

    fn exit_signal(&self) -> Option<i32> {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            self.exit_status.and_then(|s| s.signal())
        }
        #[cfg(not(unix))]
        None
    }

    fn wait_for_ready(&mut self) -> Result<(), TargetError> {
        let start = Instant::now();
        let mut backoff = CONNECT_BACKOFF_INITIAL;

        loop {
            if start.elapsed() > CONNECT_TIMEOUT {
                return Err(TargetError::ConnectionFailed(format!(
                    "redis did not become ready within {}s",
                    CONNECT_TIMEOUT.as_secs()
                )));
            }

            if !self.poll_child() {
                let stderr = self.drain_stderr();
                return Err(TargetError::ConnectionFailed(format!(
                    "redis exited during startup (status: {:?})\nstderr:\n{stderr}",
                    self.exit_status,
                )));
            }

            match TcpStream::connect_timeout(
                &std::net::SocketAddr::from((
                    self.config
                        .bind
                        .parse::<std::net::IpAddr>()
                        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
                    self.config.port,
                )),
                Duration::from_millis(100),
            ) {
                Ok(stream) => {
                    drop(stream);
                    debug!(port = self.config.port, "redis is ready");
                    return Ok(());
                }
                Err(_) => {
                    thread::sleep(backoff);
                    backoff = (backoff * 2).min(CONNECT_BACKOFF_MAX);
                }
            }
        }
    }

    fn kill_process(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };
        let pid = child.id();
        debug!(pid, "sending SIGTERM to redis");

        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        }

        if let Ok(Some(status)) = child.try_wait() {
            self.exit_status = Some(status);
        } else {
            thread::sleep(SHUTDOWN_GRACE);
            if let Ok(Some(status)) = child.try_wait() {
                self.exit_status = Some(status);
            } else {
                warn!(pid, "redis did not exit after SIGTERM, sending SIGKILL");
                let _ = child.kill();
                if let Ok(status) = child.wait() {
                    self.exit_status = Some(status);
                }
            }
        }

        self.child = None;
        self.client = None;
    }
}

impl Target for RedisTarget {
    fn execute(&mut self, script: &str) -> Result<Execution, TargetError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| TargetError::ConnectionFailed("no connection".into()))?;

        client
            .set_read_timeout(Some(self.config.exec_timeout))
            .map_err(TargetError::Io)?;

        let start = Instant::now();
        if let Err(e) = client.send_command(&["EVAL", script, "0"]) {
            if !self.poll_child() {
                self.consecutive_timeouts = 0;
                return Ok(self.classify_failure(script, start.elapsed()));
            }
            return Err(TargetError::ProtocolError(e.to_string()));
        }

        let result = client.read_response();
        let duration = start.elapsed();

        match result {
            Ok(resp) => {
                self.consecutive_timeouts = 0;
                let stderr = self.drain_stderr();
                let asan_report = self.check_sanitizer_report(&stderr);

                if asan_report.is_some() {
                    return Ok(Execution {
                        status: ExecStatus::Crash(CrashInfo {
                            signal: None,
                            asan_report,
                            script: script.to_string(),
                        }),
                        stderr,
                        duration,
                    });
                }

                let status = match resp {
                    RespValue::Error(ref msg) => ExecStatus::RuntimeError(msg.clone()),
                    _ => ExecStatus::Ok,
                };
                Ok(Execution {
                    status,
                    stderr,
                    duration,
                })
            }
            Err(crate::resp::RespError::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                self.consecutive_timeouts += 1;
                let stderr = self.drain_stderr();
                if self.consecutive_timeouts >= self.config.consecutive_timeout_threshold {
                    warn!(
                        count = self.consecutive_timeouts,
                        "consecutive timeout threshold reached, restarting"
                    );
                    self.restart()?;
                }
                Ok(Execution {
                    status: ExecStatus::Timeout,
                    stderr,
                    duration,
                })
            }
            Err(_) => {
                self.consecutive_timeouts = 0;
                Ok(self.classify_failure(script, duration))
            }
        }
    }

    fn reset(&mut self) -> Result<(), TargetError> {
        if let Some(ref mut client) = self.client {
            let _ = client.command(&["SCRIPT", "FLUSH"]);
        }
        self.shm.clear();
        Ok(())
    }

    fn restart(&mut self) -> Result<(), TargetError> {
        self.kill_process();
        self.consecutive_timeouts = 0;
        self.shm.clear();

        self.spawn_process()?;
        Ok(())
    }

    fn collect_coverage(&self) -> CoverageBitmap {
        let mut buf = CoverageBitmap::new(self.config.edge_bitmap_size, self.config.gc_bitmap_size);
        self.shm.read_into(&mut buf);
        buf
    }

    fn is_alive(&mut self) -> bool {
        self.poll_child()
    }

    fn sandbox(&self) -> &SandboxConfig {
        &self.sandbox_config
    }
}

impl Drop for RedisTarget {
    fn drop(&mut self) {
        self.kill_process();
    }
}

fn find_asan_report(lines: &[&str]) -> Option<String> {
    let mut report = Vec::new();
    let mut in_report = false;

    for line in lines {
        if line.contains("ERROR: AddressSanitizer") {
            in_report = true;
        }
        if in_report {
            report.push(*line);
            if line.contains("ABORTING") || line.contains("SUMMARY:") {
                break;
            }
        }
    }

    if report.is_empty() {
        None
    } else {
        Some(report.join("\n"))
    }
}


/// Pick an available port. Inherent TOCTOU race: the port may be taken between
/// our bind and Redis's bind. Acceptable for fuzzer workers; retry on spawn failure.
fn pick_random_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind random port");
    listener.local_addr().unwrap().port()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_asan_report_extracts_between_error_and_summary() {
        let lines = vec![
            "some redis warning",
            "=================================================================",
            "==12345==ERROR: AddressSanitizer: heap-use-after-free",
            "READ of size 8 at 0x...",
            "SUMMARY: AddressSanitizer: heap-use-after-free",
        ];
        let report = find_asan_report(&lines).unwrap();
        assert!(report.contains("AddressSanitizer"));
        assert!(report.contains("SUMMARY"));
        assert!(!report.contains("some redis warning"));
    }

    #[test]
    fn find_asan_report_returns_none_for_clean_output() {
        assert!(find_asan_report(&["normal output"]).is_none());
    }
}
