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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_crash_report_formatting() {
        let with_asan = CrashInfo {
            signal: Some(11),
            asan_report: Some("heap-use-after-free at 0x42".into()),
            ubsan_report: None,
            script: String::new(),
        };
        let report = build_crash_report(&with_asan);
        assert!(report.contains("ASan:"));
        assert!(report.contains("heap-use-after-free"));
        assert!(report.contains("Signal: 11"));

        let empty = CrashInfo {
            signal: None,
            asan_report: None,
            ubsan_report: None,
            script: String::new(),
        };
        assert_eq!(build_crash_report(&empty), "Unknown crash");
    }
}
