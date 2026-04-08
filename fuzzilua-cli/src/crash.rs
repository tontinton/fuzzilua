use std::collections::HashMap;

use fuzzilua_ir::Program;
use fuzzilua_target::CrashInfo;

pub fn build_crash_report(crash_info: &CrashInfo) -> String {
    let mut parts = Vec::new();
    if let Some(ref asan) = crash_info.asan_report {
        parts.push(format!("ASan:\n{asan}"));
    }
    if let Some(ref ubsan) = crash_info.ubsan_report {
        parts.push(format!("UBSan:\n{ubsan}"));
    }
    if let Some(sig) = crash_info.signal {
        parts.push(format!("Signal: {sig}"));
    }
    if parts.is_empty() {
        "Unknown crash".into()
    } else {
        parts.join("\n\n")
    }
}

pub fn crash_hash(crash_info: &CrashInfo) -> u64 {
    if let Some(ref asan) = crash_info.asan_report {
        let frames = extract_top_frames(asan, 3);
        if !frames.is_empty() {
            return fnv1a_64(frames.join("|").as_bytes());
        }
    }
    if let Some(ref ubsan) = crash_info.ubsan_report {
        let frames = extract_top_frames(ubsan, 3);
        if !frames.is_empty() {
            return fnv1a_64(frames.join("|").as_bytes());
        }
    }
    let sig = crash_info.signal.unwrap_or(0);
    fnv1a_64(format!("signal:{sig}").as_bytes())
}

fn extract_top_frames(report: &str, max_frames: usize) -> Vec<String> {
    let mut frames = Vec::new();
    for line in report.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#')
            && let Some(frame) = parse_frame(trimmed)
        {
            frames.push(frame);
            if frames.len() >= max_frames {
                break;
            }
        }
    }
    frames
}

fn parse_frame(line: &str) -> Option<String> {
    // ASan frames look like: #0 0x... in func_name path/to/file.c:42:5
    // We extract "func_name+offset" or just the function name
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 4 && parts[2] == "in" {
        return Some(parts[3].to_string());
    }
    // Fallback: just take the address-like part
    if parts.len() >= 2 {
        return Some(parts[1].to_string());
    }
    None
}

#[derive(Default)]
pub struct CrashDb {
    entries: HashMap<u64, CrashEntry>,
}

struct CrashEntry {
    program: Program,
}

impl CrashDb {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Returns true if this is a new unique crash.
    pub fn add(&mut self, crash_info: &CrashInfo, program: &Program) -> bool {
        let hash = crash_hash(crash_info);
        let new_size = program.instructions.len();

        match self.entries.get_mut(&hash) {
            Some(existing) => {
                if new_size < existing.program.instructions.len() {
                    existing.program = program.clone();
                }
                false
            }
            None => {
                self.entries.insert(
                    hash,
                    CrashEntry {
                        program: program.clone(),
                    },
                );
                true
            }
        }
    }

    pub fn unique_count(&self) -> usize {
        self.entries.len()
    }

    pub fn smallest_reproducer(&self, hash: u64) -> Option<&Program> {
        self.entries.get(&hash).map(|e| &e.program)
    }
}

fn fnv1a_64(data: &[u8]) -> u64 {
    const BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;
    let mut hash = BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuzzilua_ir::{Instruction, Op, Program};

    fn make_crash(signal: Option<i32>, asan: Option<&str>) -> CrashInfo {
        CrashInfo {
            signal,
            asan_report: asan.map(String::from),
            ubsan_report: None,
            script: String::new(),
        }
    }

    fn make_program(n: usize) -> Program {
        Program {
            instructions: (0..n)
                .map(|_| Instruction {
                    op: Op::LoadNil,
                    inputs: vec![],
                    outputs: vec![],
                })
                .collect(),
            next_var: 0,
        }
    }

    const ASAN_REPORT_A: &str = "\
==12345==ERROR: AddressSanitizer: heap-use-after-free
    #0 0x55a in func_a /src/a.c:10:5
    #1 0x55b in func_b /src/b.c:20:3
    #2 0x55c in func_c /src/c.c:30:1
SUMMARY: AddressSanitizer: heap-use-after-free";

    const ASAN_REPORT_A_DUP: &str = "\
==99999==ERROR: AddressSanitizer: heap-use-after-free
    #0 0xdead in func_a /src/a.c:11:5
    #1 0xbeef in func_b /src/b.c:21:3
    #2 0xcafe in func_c /src/c.c:31:1
SUMMARY: AddressSanitizer: heap-use-after-free";

    const ASAN_REPORT_B: &str = "\
==12345==ERROR: AddressSanitizer: stack-buffer-overflow
    #0 0x55a in other_func /src/x.c:10:5
    #1 0x55b in another_func /src/y.c:20:3
    #2 0x55c in third_func /src/z.c:30:1
SUMMARY: AddressSanitizer: stack-buffer-overflow";

    #[test]
    fn asan_dedup_by_function_name_ignoring_address() {
        let mut db = CrashDb::new();
        for i in 0..5 {
            db.add(
                &make_crash(Some(11), Some(ASAN_REPORT_A)),
                &make_program(10 + i),
            );
        }
        db.add(
            &make_crash(Some(11), Some(ASAN_REPORT_A_DUP)),
            &make_program(5),
        );
        assert_eq!(db.unique_count(), 1, "same func names = same crash");
    }

    #[test]
    fn different_stacks_separate() {
        let mut db = CrashDb::new();
        let crash_a = make_crash(Some(11), Some(ASAN_REPORT_A));
        let crash_b = make_crash(Some(11), Some(ASAN_REPORT_B));
        let crash_c = make_crash(Some(6), None); // signal only
        db.add(&crash_a, &make_program(10));
        db.add(&crash_b, &make_program(10));
        db.add(&crash_c, &make_program(10));
        assert_eq!(db.unique_count(), 3);
    }

    #[test]
    fn signal_only_deduped() {
        let mut db = CrashDb::new();
        let crash = make_crash(Some(6), None);
        db.add(&crash, &make_program(10));
        db.add(&crash, &make_program(5));
        assert_eq!(db.unique_count(), 1);
    }

    #[test]
    fn keeps_smallest_reproducer() {
        let mut db = CrashDb::new();
        let crash = make_crash(Some(11), Some(ASAN_REPORT_A));
        let big = make_program(20);
        let small = make_program(5);
        let medium = make_program(10);

        db.add(&crash, &big);
        db.add(&crash, &small);
        db.add(&crash, &medium);

        let hash = crash_hash(&crash);
        let best = db.smallest_reproducer(hash).unwrap();
        assert_eq!(best.instructions.len(), 5);
    }
}
