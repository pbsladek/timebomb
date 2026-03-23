use crate::error::{Error, Result};
use crate::output::OutputFormat;
use crate::report::{Report, ReportAnnotation};
use colored::Colorize;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Summary of how annotation debt has changed between two report snapshots.
#[derive(Debug, Serialize)]
pub struct TrendResult {
    pub from_timestamp: String,
    pub to_timestamp: String,
    /// Positive = more expired (worse), negative = fewer (better).
    pub expired_delta: i64,
    pub expiring_soon_delta: i64,
    pub total_delta: i64,
    /// Annotations in B.expired whose file:line key is not in A.expired.
    pub newly_expired: Vec<ReportAnnotation>,
    /// Annotations in A.expired whose file:line key is absent from B entirely.
    pub resolved: Vec<ReportAnnotation>,
    /// Annotations in A.expired that are now in B.expiring_soon (deadline bumped).
    pub snoozed: Vec<ReportAnnotation>,
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn load_report(path: &Path) -> Result<Report> {
    let content = std::fs::read_to_string(path).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;
    serde_json::from_str(&content).map_err(|e| Error::Io {
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        path: Some(path.to_path_buf()),
    })
}

/// A simple key identifying a unique annotation location.
fn annotation_key(a: &ReportAnnotation) -> String {
    format!("{}:{}", a.file, a.line)
}

fn key_set(anns: &[ReportAnnotation]) -> HashSet<String> {
    anns.iter().map(annotation_key).collect()
}

// ─── Core computation ─────────────────────────────────────────────────────────

pub fn compute_trend(a: &Report, b: &Report) -> TrendResult {
    let a_expired_keys = key_set(&a.expired);
    let b_expired_keys = key_set(&b.expired);
    let b_expiring_soon_keys = key_set(&b.expiring_soon);
    let b_ok_keys = key_set(&b.ok);

    // All keys that exist anywhere in B.
    // Use &str references into the already-owned keys to avoid cloning them again.
    let b_all_keys: HashSet<&str> = b_expired_keys
        .iter()
        .chain(b_expiring_soon_keys.iter())
        .chain(b_ok_keys.iter())
        .map(String::as_str)
        .collect();

    let newly_expired: Vec<ReportAnnotation> = b
        .expired
        .iter()
        .filter(|ann| !a_expired_keys.contains(&annotation_key(ann)))
        .cloned()
        .collect();

    let resolved: Vec<ReportAnnotation> = a
        .expired
        .iter()
        .filter(|ann| !b_all_keys.contains(annotation_key(ann).as_str()))
        .cloned()
        .collect();

    let snoozed: Vec<ReportAnnotation> = a
        .expired
        .iter()
        .filter(|ann| b_expiring_soon_keys.contains(&annotation_key(ann)))
        .cloned()
        .collect();

    let expired_delta = b.expired.len() as i64 - a.expired.len() as i64;
    let expiring_soon_delta = b.expiring_soon.len() as i64 - a.expiring_soon.len() as i64;
    let a_total = (a.expired.len() + a.expiring_soon.len() + a.ok.len()) as i64;
    let b_total = (b.expired.len() + b.expiring_soon.len() + b.ok.len()) as i64;
    let total_delta = b_total - a_total;

    TrendResult {
        from_timestamp: a.generated_at.clone(),
        to_timestamp: b.generated_at.clone(),
        expired_delta,
        expiring_soon_delta,
        total_delta,
        newly_expired,
        resolved,
        snoozed,
    }
}

// ─── Output ───────────────────────────────────────────────────────────────────

fn color_enabled() -> bool {
    std::env::var("NO_COLOR").is_err()
}

fn fmt_delta(delta: i64, use_color: bool) -> String {
    let s = if delta > 0 {
        format!("+{}", delta)
    } else {
        format!("{}", delta)
    };
    if use_color {
        if delta > 0 {
            s.red().to_string()
        } else if delta < 0 {
            s.green().to_string()
        } else {
            s
        }
    } else {
        s
    }
}

pub fn print_trend(trend: &TrendResult, format: &OutputFormat) {
    match format {
        OutputFormat::Json => match serde_json::to_string_pretty(trend) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("error serializing trend: {}", e),
        },
        OutputFormat::GitHub => print_trend_github(trend),
        OutputFormat::Terminal => print_trend_terminal(trend),
    }
}

fn print_trend_terminal(trend: &TrendResult) {
    let use_color = color_enabled();

    println!("Trend: {} → {}", trend.from_timestamp, trend.to_timestamp);
    println!();

    // Helper to compute "was X, now Y"
    let a_expired = (trend.expired_delta.unsigned_abs()) as i64;
    let _ = a_expired; // not directly available; we show delta + counts from items
                       // We don't have the absolute counts in TrendResult, so show what we have.
    println!(
        "  Expired:       {}",
        fmt_delta(trend.expired_delta, use_color)
    );
    println!(
        "  Expiring soon: {}",
        fmt_delta(trend.expiring_soon_delta, use_color)
    );
    println!(
        "  Total:         {}",
        fmt_delta(trend.total_delta, use_color)
    );
    println!();

    // Newly expired
    let n = trend.newly_expired.len();
    let header = format!("  Newly expired ({}):", n);
    if use_color && n > 0 {
        println!("{}", header.red().bold());
    } else {
        println!("{}", header);
    }
    if trend.newly_expired.is_empty() {
        println!("    (none)");
    } else {
        for ann in &trend.newly_expired {
            let line = format!(
                "    {}:{}  {}[{}]  {}",
                ann.file, ann.line, ann.tag, ann.date, ann.message
            );
            if use_color {
                println!("{}", line.red());
            } else {
                println!("{}", line);
            }
        }
    }
    println!();

    // Resolved
    let n = trend.resolved.len();
    let header = format!("  Resolved ({}):", n);
    if use_color && n > 0 {
        println!("{}", header.green().bold());
    } else {
        println!("{}", header);
    }
    if trend.resolved.is_empty() {
        println!("    (none)");
    } else {
        for ann in &trend.resolved {
            let line = format!(
                "    {}:{}  {}[{}]  (removed)",
                ann.file, ann.line, ann.tag, ann.date
            );
            if use_color {
                println!("{}", line.green());
            } else {
                println!("{}", line);
            }
        }
    }
    println!();

    // Snoozed
    let n = trend.snoozed.len();
    println!("  Snoozed ({}):", n);
    if trend.snoozed.is_empty() {
        println!("    (none)");
    } else {
        for ann in &trend.snoozed {
            println!(
                "    {}:{}  {}[{}]  {}",
                ann.file, ann.line, ann.tag, ann.date, ann.message
            );
        }
    }
}

fn print_trend_github(trend: &TrendResult) {
    for ann in &trend.newly_expired {
        println!(
            "::error file={},line={}::{} expired on {}: {}",
            ann.file, ann.line, ann.tag, ann.date, ann.message
        );
    }
    for ann in &trend.resolved {
        println!(
            "::notice file={},line={}::{} annotation resolved (removed from codebase)",
            ann.file, ann.line, ann.tag
        );
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run_trend(report_a_path: &Path, report_b_path: &Path, format: &OutputFormat) -> Result<i32> {
    let a = load_report(report_a_path)?;
    let b = load_report(report_b_path)?;
    let trend = compute_trend(&a, &b);
    print_trend(&trend, format);
    Ok(0)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;

    fn make_report_ann(file: &str, line: usize, tag: &str, date: &str) -> ReportAnnotation {
        ReportAnnotation {
            file: file.to_string(),
            line,
            tag: tag.to_string(),
            date: date.to_string(),
            owner: None,
            message: format!("message at {}:{}", file, line),
            status: "expired".to_string(),
        }
    }

    fn make_report(
        generated_at: &str,
        expired: Vec<ReportAnnotation>,
        expiring_soon: Vec<ReportAnnotation>,
        ok: Vec<ReportAnnotation>,
    ) -> Report {
        let total = expired.len() + expiring_soon.len() + ok.len();
        Report {
            generated_at: generated_at.to_string(),
            scanned_files: 1,
            total_annotations: total,
            expired,
            expiring_soon,
            ok,
        }
    }

    // ── compute_trend ─────────────────────────────────────────────────────────

    #[test]
    fn test_compute_trend_newly_expired() {
        let a = make_report("2025-01-01T00:00:00Z", vec![], vec![], vec![]);
        // B has one expired annotation that wasn't in A.expired
        let ann = make_report_ann("src/foo.rs", 10, "TODO", "2025-01-15");
        let b = make_report("2025-02-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.newly_expired.len(), 1);
        assert_eq!(trend.newly_expired[0].file, "src/foo.rs");
        assert_eq!(trend.expired_delta, 1);
    }

    #[test]
    fn test_compute_trend_resolved() {
        // Annotation was expired in A, gone in B.
        let ann = make_report_ann("src/old.rs", 5, "FIXME", "2020-12-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.resolved.len(), 1);
        assert_eq!(trend.resolved[0].file, "src/old.rs");
        assert_eq!(trend.expired_delta, -1);
    }

    #[test]
    fn test_compute_trend_snoozed() {
        // Annotation was in A.expired, now it's in B.expiring_soon (date bumped).
        let ann = make_report_ann("src/worker.rs", 88, "TODO", "2025-03-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        // Same file:line, now in expiring_soon bucket.
        let mut snoozed_ann = ann.clone();
        snoozed_ann.date = "2026-06-01".to_string();
        snoozed_ann.status = "expiring_soon".to_string();
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![snoozed_ann], vec![]);

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.snoozed.len(), 1);
        assert_eq!(trend.snoozed[0].file, "src/worker.rs");
    }

    #[test]
    fn test_compute_trend_delta_math() {
        let ann1 = make_report_ann("src/a.rs", 1, "TODO", "2020-01-01");
        let ann2 = make_report_ann("src/b.rs", 2, "FIXME", "2020-06-01");
        let ann3 = make_report_ann("src/c.rs", 3, "HACK", "2021-01-01");

        let mut soon_ann = ann1.clone();
        soon_ann.status = "expiring_soon".to_string();

        let a = make_report(
            "2025-01-01T00:00:00Z",
            vec![ann1.clone(), ann2.clone()],
            vec![soon_ann.clone()],
            vec![],
        );

        // B has only 1 expired and 0 expiring soon
        let b = make_report("2025-02-01T00:00:00Z", vec![ann3.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        // expired: was 2, now 1 → delta = -1
        assert_eq!(trend.expired_delta, -1);
        // expiring_soon: was 1, now 0 → delta = -1
        assert_eq!(trend.expiring_soon_delta, -1);
        // total: was 3 (2+1+0), now 1 → delta = -2
        assert_eq!(trend.total_delta, -2);
    }

    #[test]
    fn test_compute_trend_timestamps() {
        let a = make_report("2025-01-01T00:00:00Z", vec![], vec![], vec![]);
        let b = make_report("2025-03-15T12:00:00Z", vec![], vec![], vec![]);
        let trend = compute_trend(&a, &b);
        assert_eq!(trend.from_timestamp, "2025-01-01T00:00:00Z");
        assert_eq!(trend.to_timestamp, "2025-03-15T12:00:00Z");
    }

    #[test]
    fn test_compute_trend_no_change() {
        let ann = make_report_ann("src/x.rs", 7, "TODO", "2020-01-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let trend = compute_trend(&a, &b);
        assert_eq!(trend.expired_delta, 0);
        assert!(trend.newly_expired.is_empty());
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }

    // ── print_trend smoke tests ───────────────────────────────────────────────

    fn make_nontrivial_trend() -> TrendResult {
        TrendResult {
            from_timestamp: "2025-01-01T00:00:00Z".to_string(),
            to_timestamp: "2025-02-01T00:00:00Z".to_string(),
            expired_delta: 2,
            expiring_soon_delta: -1,
            total_delta: 1,
            newly_expired: vec![make_report_ann("src/foo.rs", 42, "TODO", "2026-01-15")],
            resolved: vec![make_report_ann("src/old.rs", 5, "TODO", "2025-12-01")],
            snoozed: vec![],
        }
    }

    #[test]
    fn test_print_trend_terminal_does_not_panic() {
        let trend = make_nontrivial_trend();
        print_trend(&trend, &OutputFormat::Terminal);
    }

    #[test]
    fn test_print_trend_json_does_not_panic() {
        let trend = make_nontrivial_trend();
        print_trend(&trend, &OutputFormat::Json);
    }

    #[test]
    fn test_print_trend_github_does_not_panic() {
        let trend = make_nontrivial_trend();
        print_trend(&trend, &OutputFormat::GitHub);
    }

    // ── run_trend (filesystem round-trip) ────────────────────────────────────

    #[test]
    fn test_run_trend_reads_json_files() {
        use crate::annotation::{Annotation, Status};
        use crate::report::{build_report, write_report};
        use crate::scanner::ScanResult;
        use chrono::NaiveDate;
        use std::path::PathBuf;

        let tmp = tempfile::tempdir().unwrap();

        let make_ann = |file: &str, line: usize, date_str: &str, status: Status| Annotation {
            file: PathBuf::from(file),
            line,
            tag: "TODO".to_string(),
            date: NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap(),
            owner: None,
            message: "test".to_string(),
            status,
            blamed_owner: None,
        };

        let result_a = ScanResult {
            annotations: vec![make_ann("src/a.rs", 1, "2020-01-01", Status::Expired)],
            scanned_files: 1,
            skipped_files: 0,
        };
        let report_a = build_report(&result_a, "2025-01-01T00:00:00Z");
        let path_a = tmp.path().join("report_a.json");
        write_report(&report_a, &path_a).unwrap();

        let result_b = ScanResult {
            annotations: vec![make_ann("src/b.rs", 2, "2021-06-01", Status::Expired)],
            scanned_files: 1,
            skipped_files: 0,
        };
        let report_b = build_report(&result_b, "2025-02-01T00:00:00Z");
        let path_b = tmp.path().join("report_b.json");
        write_report(&report_b, &path_b).unwrap();

        let code = run_trend(&path_a, &path_b, &OutputFormat::Json).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_trend_error_on_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does_not_exist.json");
        // Both paths missing — should return an Err, not panic.
        let result = run_trend(&missing, &missing, &OutputFormat::Terminal);
        assert!(result.is_err());
    }

    // ── annotation_key format ─────────────────────────────────────────────────

    #[test]
    fn test_annotation_key_format() {
        let ann = make_report_ann("src/lib.rs", 42, "TODO", "2025-01-01");
        assert_eq!(annotation_key(&ann), "src/lib.rs:42");
    }

    // ── same annotation stays in neither newly_expired nor resolved ───────────

    #[test]
    fn test_compute_trend_unchanged_expired_is_neither_new_nor_resolved() {
        // Same file:line is expired in both A and B → no change.
        let ann = make_report_ann("src/x.rs", 10, "TODO", "2020-01-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert!(trend.newly_expired.is_empty(), "not newly expired");
        assert!(trend.resolved.is_empty(), "not resolved");
        assert!(trend.snoozed.is_empty(), "not snoozed");
        assert_eq!(trend.expired_delta, 0);
    }

    // ── multiple snoozed ─────────────────────────────────────────────────────

    #[test]
    fn test_compute_trend_multiple_snoozed() {
        let ann1 = make_report_ann("src/a.rs", 1, "TODO", "2025-01-01");
        let ann2 = make_report_ann("src/b.rs", 2, "FIXME", "2025-02-01");
        let a = make_report(
            "2025-01-01T00:00:00Z",
            vec![ann1.clone(), ann2.clone()],
            vec![],
            vec![],
        );

        let mut snoozed1 = ann1.clone();
        snoozed1.date = "2026-12-01".to_string();
        snoozed1.status = "expiring_soon".to_string();
        let mut snoozed2 = ann2.clone();
        snoozed2.date = "2027-06-01".to_string();
        snoozed2.status = "expiring_soon".to_string();

        let b = make_report(
            "2025-02-01T00:00:00Z",
            vec![],
            vec![snoozed1, snoozed2],
            vec![],
        );

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.snoozed.len(), 2);
        assert_eq!(trend.expired_delta, -2);
    }

    // ── resolved vs ok (annotation moved to ok, not just expiring_soon) ──────

    #[test]
    fn test_compute_trend_moved_to_ok_is_resolved() {
        // Annotation was expired in A; now it's in B.ok (date bumped far out).
        let ann = make_report_ann("src/z.rs", 99, "HACK", "2020-05-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let mut ok_ann = ann.clone();
        ok_ann.date = "2099-01-01".to_string();
        ok_ann.status = "ok".to_string();
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![ok_ann]);

        let trend = compute_trend(&a, &b);
        // Still present in B (as ok) → not resolved, not snoozed.
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }

    // ── empty reports ─────────────────────────────────────────────────────────

    #[test]
    fn test_compute_trend_both_empty() {
        let a = make_report("2025-01-01T00:00:00Z", vec![], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![]);
        let trend = compute_trend(&a, &b);
        assert_eq!(trend.expired_delta, 0);
        assert_eq!(trend.expiring_soon_delta, 0);
        assert_eq!(trend.total_delta, 0);
        assert!(trend.newly_expired.is_empty());
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }
}
