use crate::error::{Error, Result};
use crate::output::OutputFormat;
use crate::report::{Report, ReportAnnotation};
use colored::Colorize;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Summary of how fuse debt has changed between two report snapshots.
#[derive(Debug, Serialize)]
pub struct TrendResult {
    pub from_timestamp: String,
    pub to_timestamp: String,
    /// Positive = more detonated (worse), negative = fewer (better).
    pub detonated_delta: i64,
    pub ticking_delta: i64,
    pub total_delta: i64,
    /// Fuses in B.detonated whose file:line key is not in A.detonated.
    pub newly_detonated: Vec<ReportAnnotation>,
    /// Fuses in A.detonated whose file:line key is absent from B entirely.
    pub resolved: Vec<ReportAnnotation>,
    /// Fuses in A.detonated that are now in B.ticking (deadline bumped).
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

/// A simple key identifying a unique fuse location.
fn annotation_key(a: &ReportAnnotation) -> String {
    format!("{}:{}", a.file, a.line)
}

fn key_set(anns: &[ReportAnnotation]) -> HashSet<String> {
    anns.iter().map(annotation_key).collect()
}

// ─── Core computation ─────────────────────────────────────────────────────────

pub fn compute_trend(a: &Report, b: &Report) -> TrendResult {
    let a_detonated_keys = key_set(&a.detonated);
    let b_detonated_keys = key_set(&b.detonated);
    let b_ticking_keys = key_set(&b.ticking);
    let b_inert_keys = key_set(&b.inert);

    // All keys that exist anywhere in B.
    // Use &str references into the already-owned keys to avoid cloning them again.
    let b_all_keys: HashSet<&str> = b_detonated_keys
        .iter()
        .chain(b_ticking_keys.iter())
        .chain(b_inert_keys.iter())
        .map(String::as_str)
        .collect();

    let newly_detonated: Vec<ReportAnnotation> = b
        .detonated
        .iter()
        .filter(|ann| !a_detonated_keys.contains(&annotation_key(ann)))
        .cloned()
        .collect();

    let resolved: Vec<ReportAnnotation> = a
        .detonated
        .iter()
        .filter(|ann| !b_all_keys.contains(annotation_key(ann).as_str()))
        .cloned()
        .collect();

    let snoozed: Vec<ReportAnnotation> = a
        .detonated
        .iter()
        .filter(|ann| b_ticking_keys.contains(&annotation_key(ann)))
        .cloned()
        .collect();

    let detonated_delta = b.detonated.len() as i64 - a.detonated.len() as i64;
    let ticking_delta = b.ticking.len() as i64 - a.ticking.len() as i64;
    let a_total = (a.detonated.len() + a.ticking.len() + a.inert.len()) as i64;
    let b_total = (b.detonated.len() + b.ticking.len() + b.inert.len()) as i64;
    let total_delta = b_total - a_total;

    TrendResult {
        from_timestamp: a.generated_at.clone(),
        to_timestamp: b.generated_at.clone(),
        detonated_delta,
        ticking_delta,
        total_delta,
        newly_detonated,
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
        OutputFormat::Terminal | OutputFormat::Csv => print_trend_terminal(trend),
    }
}

fn print_trend_terminal(trend: &TrendResult) {
    let use_color = color_enabled();

    println!("Trend: {} → {}", trend.from_timestamp, trend.to_timestamp);
    println!();

    println!(
        "  Detonated:    {}",
        fmt_delta(trend.detonated_delta, use_color)
    );
    println!(
        "  Ticking:      {}",
        fmt_delta(trend.ticking_delta, use_color)
    );
    println!(
        "  Total:        {}",
        fmt_delta(trend.total_delta, use_color)
    );
    println!();

    // Newly detonated
    let n = trend.newly_detonated.len();
    let header = format!("  Newly detonated ({}):", n);
    if use_color && n > 0 {
        println!("{}", header.red().bold());
    } else {
        println!("{}", header);
    }
    if trend.newly_detonated.is_empty() {
        println!("    (none)");
    } else {
        for ann in &trend.newly_detonated {
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
    for ann in &trend.newly_detonated {
        println!(
            "::error file={},line={}::{} detonated on {}: {}",
            ann.file, ann.line, ann.tag, ann.date, ann.message
        );
    }
    for ann in &trend.resolved {
        println!(
            "::notice file={},line={}::{} fuse resolved (removed from codebase)",
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
            status: "detonated".to_string(),
        }
    }

    fn make_report(
        generated_at: &str,
        detonated: Vec<ReportAnnotation>,
        ticking: Vec<ReportAnnotation>,
        inert: Vec<ReportAnnotation>,
    ) -> Report {
        let total = detonated.len() + ticking.len() + inert.len();
        Report {
            generated_at: generated_at.to_string(),
            swept_files: 1,
            total_fuses: total,
            detonated,
            ticking,
            inert,
        }
    }

    // ── compute_trend ─────────────────────────────────────────────────────────

    #[test]
    fn test_compute_trend_newly_detonated() {
        let a = make_report("2025-01-01T00:00:00Z", vec![], vec![], vec![]);
        // B has one detonated fuse that wasn't in A.detonated
        let ann = make_report_ann("src/foo.rs", 10, "TODO", "2025-01-15");
        let b = make_report("2025-02-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.newly_detonated.len(), 1);
        assert_eq!(trend.newly_detonated[0].file, "src/foo.rs");
        assert_eq!(trend.detonated_delta, 1);
    }

    #[test]
    fn test_compute_trend_resolved() {
        // Fuse was detonated in A, gone in B.
        let ann = make_report_ann("src/old.rs", 5, "FIXME", "2020-12-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.resolved.len(), 1);
        assert_eq!(trend.resolved[0].file, "src/old.rs");
        assert_eq!(trend.detonated_delta, -1);
    }

    #[test]
    fn test_compute_trend_snoozed() {
        // Fuse was in A.detonated, now it's in B.ticking (date bumped).
        let ann = make_report_ann("src/worker.rs", 88, "TODO", "2025-03-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        // Same file:line, now in ticking bucket.
        let mut snoozed_ann = ann.clone();
        snoozed_ann.date = "2026-06-01".to_string();
        snoozed_ann.status = "ticking".to_string();
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

        let mut ticking_ann = ann1.clone();
        ticking_ann.status = "ticking".to_string();

        let a = make_report(
            "2025-01-01T00:00:00Z",
            vec![ann1.clone(), ann2.clone()],
            vec![ticking_ann.clone()],
            vec![],
        );

        // B has only 1 detonated and 0 ticking
        let b = make_report("2025-02-01T00:00:00Z", vec![ann3.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        // detonated: was 2, now 1 → delta = -1
        assert_eq!(trend.detonated_delta, -1);
        // ticking: was 1, now 0 → delta = -1
        assert_eq!(trend.ticking_delta, -1);
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
        assert_eq!(trend.detonated_delta, 0);
        assert!(trend.newly_detonated.is_empty());
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }

    // ── print_trend smoke tests ───────────────────────────────────────────────

    fn make_nontrivial_trend() -> TrendResult {
        TrendResult {
            from_timestamp: "2025-01-01T00:00:00Z".to_string(),
            to_timestamp: "2025-02-01T00:00:00Z".to_string(),
            detonated_delta: 2,
            ticking_delta: -1,
            total_delta: 1,
            newly_detonated: vec![make_report_ann("src/foo.rs", 42, "TODO", "2026-01-15")],
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
        use crate::annotation::{Fuse, Status};
        use crate::report::{build_report, write_report};
        use crate::scanner::ScanResult;
        use chrono::NaiveDate;
        use std::path::PathBuf;

        let tmp = tempfile::tempdir().unwrap();

        let make_fuse = |file: &str, line: usize, date_str: &str, status: Status| Fuse {
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
            fuses: vec![make_fuse("src/a.rs", 1, "2020-01-01", Status::Detonated)],
            swept_files: 1,
            skipped_files: 0,
        };
        let report_a = build_report(&result_a, "2025-01-01T00:00:00Z");
        let path_a = tmp.path().join("report_a.json");
        write_report(&report_a, &path_a).unwrap();

        let result_b = ScanResult {
            fuses: vec![make_fuse("src/b.rs", 2, "2021-06-01", Status::Detonated)],
            swept_files: 1,
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

    // ── same fuse stays in neither newly_detonated nor resolved ──────────────

    #[test]
    fn test_compute_trend_unchanged_detonated_is_neither_new_nor_resolved() {
        // Same file:line is detonated in both A and B → no change.
        let ann = make_report_ann("src/x.rs", 10, "TODO", "2020-01-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let trend = compute_trend(&a, &b);
        assert!(trend.newly_detonated.is_empty(), "not newly detonated");
        assert!(trend.resolved.is_empty(), "not resolved");
        assert!(trend.snoozed.is_empty(), "not snoozed");
        assert_eq!(trend.detonated_delta, 0);
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
        snoozed1.status = "ticking".to_string();
        let mut snoozed2 = ann2.clone();
        snoozed2.date = "2027-06-01".to_string();
        snoozed2.status = "ticking".to_string();

        let b = make_report(
            "2025-02-01T00:00:00Z",
            vec![],
            vec![snoozed1, snoozed2],
            vec![],
        );

        let trend = compute_trend(&a, &b);
        assert_eq!(trend.snoozed.len(), 2);
        assert_eq!(trend.detonated_delta, -2);
    }

    // ── resolved vs inert (fuse moved to inert, not just ticking) ────────────

    #[test]
    fn test_compute_trend_moved_to_inert_is_resolved() {
        // Fuse was detonated in A; now it's in B.inert (date bumped far out).
        let ann = make_report_ann("src/z.rs", 99, "HACK", "2020-05-01");
        let a = make_report("2025-01-01T00:00:00Z", vec![ann.clone()], vec![], vec![]);

        let mut inert_ann = ann.clone();
        inert_ann.date = "2099-01-01".to_string();
        inert_ann.status = "inert".to_string();
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![inert_ann]);

        let trend = compute_trend(&a, &b);
        // Still present in B (as inert) → not resolved, not snoozed.
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }

    // ── empty reports ─────────────────────────────────────────────────────────

    #[test]
    fn test_compute_trend_both_empty() {
        let a = make_report("2025-01-01T00:00:00Z", vec![], vec![], vec![]);
        let b = make_report("2025-02-01T00:00:00Z", vec![], vec![], vec![]);
        let trend = compute_trend(&a, &b);
        assert_eq!(trend.detonated_delta, 0);
        assert_eq!(trend.ticking_delta, 0);
        assert_eq!(trend.total_delta, 0);
        assert!(trend.newly_detonated.is_empty());
        assert!(trend.resolved.is_empty());
        assert!(trend.snoozed.is_empty());
    }
}
