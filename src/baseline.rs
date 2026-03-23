//! Baseline save/show/ratchet enforcement for `timebomb baseline` subcommand.
//!
//! "today" is always injected — never fetched internally.
//! "generated_at" is always injected from main.rs — never fetched internally.

use crate::config::Config;
use crate::error::{Error, Result};
use crate::scanner::scan;
use chrono::NaiveDate;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The baseline snapshot stored in `.timebomb-baseline.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// RFC 3339 timestamp of when this baseline was saved (set by the caller in main.rs).
    pub generated_at: String,
    /// Number of expired annotations at the time the baseline was saved.
    pub expired: usize,
    /// Number of expiring-soon annotations at the time the baseline was saved.
    pub expiring_soon: usize,
}

/// Load the baseline from a JSON file.
///
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Err` if the file exists but cannot be read or parsed.
pub fn load_baseline(path: &Path) -> Result<Option<Baseline>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;
    let baseline: Baseline = serde_json::from_str(&content).map_err(|e| {
        Error::InvalidArgument(format!(
            "failed to parse baseline file '{}': {}",
            path.display(),
            e
        ))
    })?;
    Ok(Some(baseline))
}

/// Write a baseline to a JSON file.
pub fn save_baseline(baseline: &Baseline, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(baseline)
        .map_err(|e| Error::InvalidArgument(format!("failed to serialize baseline: {}", e)))?;
    std::fs::write(path, json).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })
}

/// Scan, count, write baseline, print confirmation. Returns exit code 0 on success.
pub fn run_baseline_save(
    scan_path: &Path,
    cfg: &Config,
    today: NaiveDate,
    baseline_path: &Path,
    generated_at: &str,
) -> Result<i32> {
    let result = scan(scan_path, cfg, today)?;
    let expired = result.expired().len();
    let expiring_soon = result.expiring_soon().len();

    let baseline = Baseline {
        generated_at: generated_at.to_string(),
        expired,
        expiring_soon,
    };

    save_baseline(&baseline, baseline_path)?;

    println!(
        "baseline saved to '{}': expired={}, expiring_soon={}",
        baseline_path.display(),
        expired,
        expiring_soon
    );

    Ok(0)
}

/// Scan, load baseline (if any), print comparison table. Returns exit code 0 on success.
pub fn run_baseline_show(
    scan_path: &Path,
    cfg: &Config,
    today: NaiveDate,
    baseline_path: &Path,
) -> Result<i32> {
    let result = scan(scan_path, cfg, today)?;
    let current_expired = result.expired().len();
    let current_expiring_soon = result.expiring_soon().len();

    let baseline = load_baseline(baseline_path)?;

    match baseline {
        None => {
            println!("{:>21}  (no baseline saved)", "current");
            println!("{:<16} {:>7}", "expired", current_expired);
            println!("{:<16} {:>7}", "expiring_soon", current_expiring_soon);
        }
        Some(ref b) => {
            // Print table header
            println!("{:>21}  {:>8}", "current", "baseline");

            // expired row — highlight in red if current exceeds baseline
            let expired_current_str = current_expired.to_string();
            let expired_baseline_str = b.expired.to_string();
            if current_expired > b.expired {
                println!(
                    "{:<16} {:>7}  {:>8}",
                    "expired",
                    expired_current_str.red().bold(),
                    expired_baseline_str
                );
            } else {
                println!(
                    "{:<16} {:>7}  {:>8}",
                    "expired", expired_current_str, expired_baseline_str
                );
            }

            // expiring_soon row — highlight in red if current exceeds baseline
            let soon_current_str = current_expiring_soon.to_string();
            let soon_baseline_str = b.expiring_soon.to_string();
            if current_expiring_soon > b.expiring_soon {
                println!(
                    "{:<16} {:>7}  {:>8}",
                    "expiring_soon",
                    soon_current_str.red().bold(),
                    soon_baseline_str
                );
            } else {
                println!(
                    "{:<16} {:>7}  {:>8}",
                    "expiring_soon", soon_current_str, soon_baseline_str
                );
            }
        }
    }

    Ok(0)
}

/// Pure ratchet check — no I/O.
///
/// All four constraints are checked independently and all violations are reported.
/// Returns an empty vec if no violations are found.
pub fn check_ratchet(
    expired: usize,
    expiring_soon: usize,
    baseline: Option<&Baseline>,
    max_expired: Option<usize>,
    max_expiring_soon: Option<usize>,
) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();

    // Config limit checks
    if let Some(limit) = max_expired {
        if expired > limit {
            violations.push(format!(
                "expired count {} exceeds max_expired limit of {}",
                expired, limit
            ));
        }
    }

    if let Some(limit) = max_expiring_soon {
        if expiring_soon > limit {
            violations.push(format!(
                "expiring_soon count {} exceeds max_expiring_soon limit of {}",
                expiring_soon, limit
            ));
        }
    }

    // Baseline ratchet checks
    if let Some(b) = baseline {
        if expired > b.expired {
            violations.push(format!(
                "expired count {} exceeds baseline of {} — ratchet violated",
                expired, b.expired
            ));
        }
        if expiring_soon > b.expiring_soon {
            violations.push(format!(
                "expiring_soon count {} exceeds baseline of {} — ratchet violated",
                expiring_soon, b.expiring_soon
            ));
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ─── check_ratchet unit tests ─────────────────────────────────────────────

    #[test]
    fn test_check_ratchet_no_baseline_no_max() {
        let violations = check_ratchet(5, 10, None, None, None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_ratchet_max_expired_violated() {
        let violations = check_ratchet(3, 0, None, Some(2), None);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("max_expired limit of 2"));
    }

    #[test]
    fn test_check_ratchet_max_expired_at_limit_ok() {
        let violations = check_ratchet(2, 0, None, Some(2), None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_ratchet_baseline_expired_exceeded() {
        let baseline = Baseline {
            generated_at: "2025-01-01T00:00:00Z".to_string(),
            expired: 2,
            expiring_soon: 0,
        };
        let violations = check_ratchet(3, 0, Some(&baseline), None, None);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("ratchet violated"));
        assert!(violations[0].contains("baseline of 2"));
    }

    #[test]
    fn test_check_ratchet_baseline_improved_ok() {
        let baseline = Baseline {
            generated_at: "2025-01-01T00:00:00Z".to_string(),
            expired: 5,
            expiring_soon: 0,
        };
        let violations = check_ratchet(3, 0, Some(&baseline), None, None);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_ratchet_expiring_soon_violated() {
        let violations = check_ratchet(0, 15, None, None, Some(10));
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("max_expiring_soon limit of 10"));
    }

    #[test]
    fn test_check_ratchet_multiple_violations() {
        let violations = check_ratchet(5, 20, None, Some(3), Some(10));
        assert_eq!(violations.len(), 2);
    }

    // ─── load_baseline tests ──────────────────────────────────────────────────

    #[test]
    fn test_load_baseline_nonexistent_returns_none() {
        let result = load_baseline(std::path::Path::new(
            "/nonexistent/path/.timebomb-baseline.json",
        ));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_load_baseline_invalid_json_returns_err() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "this is not valid json {{{{").unwrap();
        let result = load_baseline(f.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let baseline_path = dir.path().join("baseline.json");

        let baseline = Baseline {
            generated_at: "2025-06-01T12:00:00Z".to_string(),
            expired: 3,
            expiring_soon: 7,
        };

        save_baseline(&baseline, &baseline_path).unwrap();
        let loaded = load_baseline(&baseline_path).unwrap().unwrap();

        assert_eq!(loaded.generated_at, baseline.generated_at);
        assert_eq!(loaded.expired, baseline.expired);
        assert_eq!(loaded.expiring_soon, baseline.expiring_soon);
    }
}
