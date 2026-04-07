use std::collections::VecDeque;
use std::time::Duration;

use fuzzilua_coverage::CoverageBitmap;

use crate::{ExecStatus, Execution, SandboxConfig, Target, TargetError};

pub struct MockTarget {
    responses: VecDeque<ExecStatus>,
    coverage: Option<CoverageBitmap>,
    sandbox_config: SandboxConfig,
    pub execute_count: u64,
    pub reset_count: u64,
    pub restart_count: u64,
    alive: bool,
    edge_size: usize,
    gc_size: usize,
}

impl MockTarget {
    pub fn new(responses: Vec<ExecStatus>, edge_size: usize, gc_size: usize) -> Self {
        Self {
            responses: VecDeque::from(responses),
            coverage: None,
            sandbox_config: SandboxConfig::redis_lua51(Duration::from_secs(5)),
            execute_count: 0,
            reset_count: 0,
            restart_count: 0,
            alive: true,
            edge_size,
            gc_size,
        }
    }

    pub fn set_coverage(&mut self, coverage: CoverageBitmap) {
        self.coverage = Some(coverage);
    }

    pub fn set_alive(&mut self, alive: bool) {
        self.alive = alive;
    }
}

impl Target for MockTarget {
    fn execute(&mut self, _script: &str) -> Result<Execution, TargetError> {
        self.execute_count += 1;
        let status = self.responses.pop_front().unwrap_or(ExecStatus::Ok);
        Ok(Execution {
            status,
            stderr: String::new(),
            duration: Duration::from_millis(1),
        })
    }

    fn reset(&mut self) -> Result<(), TargetError> {
        self.reset_count += 1;
        self.coverage = None;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), TargetError> {
        self.restart_count += 1;
        self.alive = true;
        Ok(())
    }

    fn collect_coverage(&self) -> CoverageBitmap {
        match &self.coverage {
            Some(cov) => CoverageBitmap::from_raw(cov.as_bytes().to_vec(), cov.edge_len()),
            None => CoverageBitmap::new(self.edge_size, self.gc_size),
        }
    }

    fn is_alive(&mut self) -> bool {
        self.alive
    }

    fn sandbox(&self) -> &SandboxConfig {
        &self.sandbox_config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_canned_responses() {
        let mut target = MockTarget::new(
            vec![
                ExecStatus::Ok,
                ExecStatus::RuntimeError("boom".into()),
                ExecStatus::Timeout,
            ],
            64,
            64,
        );

        let r1 = target.execute("return 1").unwrap();
        assert!(matches!(r1.status, ExecStatus::Ok));

        let r2 = target.execute("error('boom')").unwrap();
        assert!(matches!(r2.status, ExecStatus::RuntimeError(ref s) if s == "boom"));

        let r3 = target.execute("while true do end").unwrap();
        assert!(matches!(r3.status, ExecStatus::Timeout));

        let r4 = target.execute("return 2").unwrap();
        assert!(matches!(r4.status, ExecStatus::Ok));

        assert_eq!(target.execute_count, 4);
    }

    #[test]
    fn mock_tracks_reset_and_restart() {
        let mut target = MockTarget::new(vec![], 64, 64);
        target.reset().unwrap();
        target.reset().unwrap();
        target.restart().unwrap();
        assert_eq!(target.reset_count, 2);
        assert_eq!(target.restart_count, 1);
    }

    #[test]
    fn mock_coverage_returns_set_bitmap() {
        let mut target = MockTarget::new(vec![], 16, 16);
        let mut bm = CoverageBitmap::new(16, 16);
        bm.edge_bytes_mut()[0] = 0xAA;
        bm.gc_bytes_mut()[0] = 0xBB;
        target.set_coverage(bm);

        let cov = target.collect_coverage();
        assert_eq!(cov.edge_bytes()[0], 0xAA);
        assert_eq!(cov.gc_bytes()[0], 0xBB);
    }

    #[test]
    fn mock_alive_flag() {
        let mut target = MockTarget::new(vec![], 64, 64);
        assert!(target.is_alive());
        target.set_alive(false);
        assert!(!target.is_alive());
        target.restart().unwrap();
        assert!(target.is_alive());
    }
}
