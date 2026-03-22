use crate::error::{Error, Result};
use crate::output::OutputFormat;
use crate::scanner::ScanResult;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ─── Core types ───────────────────────────────────────────────────────────────

/// A single annotation as stored in the persisted report file.
/// Owned strings — no lifetimes — so it can be deserialized easily.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAnnotation {
    pub file: String,
    pub line: usize,
    pub tag: String,
    pub date: String,
    pub owner: Option<String>,
    pub message: String,
    pub status: String,
}

/// The persisted report file format.
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    /// RFC 3339 timestamp of when this report was generated.
    pub generated_at: String,
    pub scanned_files: usize,
    pub total_annotations: usize,
    pub expired: Vec<ReportAnnotation>,
    pub expiring_soon: Vec<ReportAnnotation>,
    pub ok: Vec<ReportAnnotation>,
}

/// The result of diffing two reports.
#[derive(Debug)]
pub struct ReportDiff {
    /// Annotations that are expired in the new report but were not in the old one
    /// (either newly added past their deadline, or crossed the deadline since last report).
    pub new_expired: Vec<ReportAnnotation>,
    /// Annotations present in old report but absent in new (cleaned up / deleted).
    pub resolved: Vec<ReportAnnotation>,
    /// Annotations in new report that weren't in old report at all (any status).
    pub new_annotations: Vec<ReportAnnotation>,
    /// Annotations whose date changed between old and new report (snoozed).
    pub snoozed: Vec<(ReportAnnotation, ReportAnnotation)>, // (old, new)
}

// ─── Helper for JSON diff serialisation ──────────────────────────────────────

#[derive(Serialize)]
struct SnoozedPair<'a> {
    before: &'a ReportAnnotation,
    after: &'a ReportAnnotation,
}

#[derive(Serialize)]
struct DiffJson<'a> {
    new_expired: &'a [ReportAnnotation],
    resolved: &'a [ReportAnnotation],
    new_annotations: &'a [ReportAnnotation],
    snoozed: Vec<SnoozedPair<'a>>,
}

// ─── Key type used for O(1) lookup ───────────────────────────────────────────

type AnnKey = (String, usize, String);

fn ann_key(a: &ReportAnnotation) -> AnnKey {
    (a.file.clone(), a.line, a.tag.clone())
}

fn make_key_map(anns: &[ReportAnnotation]) -> HashMap<AnnKey, &ReportAnnotation> {
    anns.iter().map(|a| (ann_key(a), a)).collect()
}

/// Build a map covering all three status buckets of a report.
fn all_key_map(report: &Report) -> HashMap<AnnKey, &ReportAnnotation> {
    let mut map = HashMap::new();
    for a in &report.expired {
        map.insert(ann_key(a), a);
    }
    for a in &report.expiring_soon {
        map.insert(ann_key(a), a);
    }
    for a in &report.ok {
        map.insert(ann_key(a), a);
    }
    map
}

// ─── Public functions ─────────────────────────────────────────────────────────

/// Convert a ScanResult into a Report. `generated_at` should be an RFC 3339 string.
/// Accept it as a parameter for testability.
pub fn build_report(result: &ScanResult, generated_at: &str) -> Report {
    let to_report_ann = |a: &crate::annotation::Annotation| ReportAnnotation {
        file: a.file.display().to_string(),
        line: a.line,
        tag: a.tag.clone(),
        date: a.date_str(),
        owner: a.owner.clone(),
        message: a.message.clone(),
        status: a.status.as_str().to_string(),
    };

    let expired: Vec<ReportAnnotation> = result.expired().into_iter().map(to_report_ann).collect();
    let expiring_soon: Vec<ReportAnnotation> = result
        .expiring_soon()
        .into_iter()
        .map(to_report_ann)
        .collect();
    let ok: Vec<ReportAnnotation> = result.ok().into_iter().map(to_report_ann).collect();

    let total_annotations = expired.len() + expiring_soon.len() + ok.len();

    Report {
        generated_at: generated_at.to_string(),
        scanned_files: result.scanned_files,
        total_annotations,
        expired,
        expiring_soon,
        ok,
    }
}

/// Write a Report to a JSON file at `path`.
pub fn write_report(report: &Report, path: &Path) -> Result<()> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io {
                source: e,
                path: Some(parent.to_path_buf()),
            })?;
        }
    }

    let json = serde_json::to_string_pretty(report).map_err(|e| Error::Io {
        source: std::io::Error::other(e.to_string()),
        path: Some(path.to_path_buf()),
    })?;

    std::fs::write(path, json).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })
}

/// Read a Report from a JSON file at `path`.
/// Returns Ok(None) if the file does not exist (first run).
/// Returns Err if the file exists but cannot be parsed.
pub fn read_report(path: &Path) -> Result<Option<Report>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;

    let report: Report = serde_json::from_str(&content).map_err(|e| Error::Io {
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        path: Some(path.to_path_buf()),
    })?;

    Ok(Some(report))
}

/// Diff two reports. `old` is the previously persisted report, `new` is the freshly built one.
pub fn diff_reports(old: &Report, new: &Report) -> ReportDiff {
    let old_expired_map = make_key_map(&old.expired);
    let old_all_map = all_key_map(old);
    let new_all_map = all_key_map(new);

    // new_expired: in new.expired but key not in old.expired
    let new_expired: Vec<ReportAnnotation> = new
        .expired
        .iter()
        .filter(|a| !old_expired_map.contains_key(&ann_key(a)))
        .cloned()
        .collect();

    // resolved: key is in old.expired but not found anywhere in new
    let resolved: Vec<ReportAnnotation> = old
        .expired
        .iter()
        .filter(|a| !new_all_map.contains_key(&ann_key(a)))
        .cloned()
        .collect();

    // new_annotations: key present in new (any status) but not present anywhere in old
    let new_annotations: Vec<ReportAnnotation> = {
        let mut seen_keys = std::collections::HashSet::new();
        let mut result = Vec::new();
        for bucket in [&new.expired, &new.expiring_soon, &new.ok] {
            for a in bucket {
                let key = ann_key(a);
                if !old_all_map.contains_key(&key) && seen_keys.insert(key) {
                    result.push(a.clone());
                }
            }
        }
        result
    };

    // snoozed: key present in both old and new, but old.date != new.date
    let snoozed: Vec<(ReportAnnotation, ReportAnnotation)> = {
        let mut result = Vec::new();
        for bucket in [&new.expired, &new.expiring_soon, &new.ok] {
            for new_ann in bucket {
                let key = ann_key(new_ann);
                if let Some(old_ann) = old_all_map.get(&key) {
                    if old_ann.date != new_ann.date {
                        result.push(((*old_ann).clone(), new_ann.clone()));
                    }
                }
            }
        }
        result
    };

    ReportDiff {
        new_expired,
        resolved,
        new_annotations,
        snoozed,
    }
}

/// Whether color output should be enabled (respects NO_COLOR).
fn color_enabled() -> bool {
    std::env::var("NO_COLOR").is_err()
}

/// Print a ReportDiff to stdout in terminal format.
pub fn print_diff_terminal(diff: &ReportDiff) {
    let use_color = color_enabled();

    println!("REPORT DIFF");
    println!("-----------");

    // new expired
    let n = diff.new_expired.len();
    println!("{} new expired annotation(s):", n);
    for a in &diff.new_expired {
        let label = "EXPIRED";
        let location = format!("{}:{}", a.file, a.line);
        let tag_date = format!("{}[{}]", a.tag, a.date);
        let line = format!(
            "  {:<8} {:<30} {:<22} {}",
            label, location, tag_date, a.message
        );
        if use_color {
            println!("{}", line.red());
        } else {
            println!("{}", line);
        }
    }

    println!();

    // resolved
    let n = diff.resolved.len();
    println!("{} resolved annotation(s):", n);
    for a in &diff.resolved {
        let label = "REMOVED";
        let location = format!("{}:{}", a.file, a.line);
        let tag_date = format!("{}[{}]", a.tag, a.date);
        let line = format!(
            "  {:<8} {:<30} {:<22} {}",
            label, location, tag_date, a.message
        );
        if use_color {
            println!("{}", line.green());
        } else {
            println!("{}", line);
        }
    }

    println!();

    // new annotations
    let n = diff.new_annotations.len();
    println!("{} new annotation(s) added:", n);
    for a in &diff.new_annotations {
        let label = "NEW";
        let location = format!("{}:{}", a.file, a.line);
        let tag_date = format!("{}[{}]", a.tag, a.date);
        let line = format!(
            "  {:<8} {:<30} {:<22} {}",
            label, location, tag_date, a.message
        );
        if use_color {
            println!("{}", line.yellow());
        } else {
            println!("{}", line);
        }
    }

    println!();

    // snoozed
    let n = diff.snoozed.len();
    println!("{} snoozed annotation(s):", n);
    for (old, new) in &diff.snoozed {
        let label = "SNOOZED";
        let location = format!("{}:{}", new.file, new.line);
        let tag_date = format!("{}[{}→{}]", new.tag, old.date, new.date);
        let line = format!(
            "  {:<8} {:<30} {:<22} {}",
            label, location, tag_date, new.message
        );
        if use_color {
            println!("{}", line.cyan());
        } else {
            println!("{}", line);
        }
    }
}

/// Print a ReportDiff as JSON to stdout.
pub fn print_diff_json(diff: &ReportDiff) {
    let snoozed: Vec<SnoozedPair<'_>> = diff
        .snoozed
        .iter()
        .map(|(old, new)| SnoozedPair {
            before: old,
            after: new,
        })
        .collect();

    let payload = DiffJson {
        new_expired: &diff.new_expired,
        resolved: &diff.resolved,
        new_annotations: &diff.new_annotations,
        snoozed,
    };

    match serde_json::to_string_pretty(&payload) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("error serializing diff: {}", e),
    }
}

/// Top-level entry point — called from main.rs.
/// - Builds the new report from the scan result.
/// - If `diff` is true and a previous report file exists, compute and print the diff.
/// - Always writes the new report to `out_path`.
/// - Returns exit code: 0 normally, 1 if `fail_on_new` is true and `diff.new_expired` is non-empty.
pub fn run_report(
    result: &ScanResult,
    out_path: &Path,
    diff: bool,
    fail_on_new: bool,
    format: &OutputFormat,
    generated_at: &str,
) -> Result<i32> {
    let new_report = build_report(result, generated_at);

    let mut exit_code = 0i32;

    if diff {
        match read_report(out_path)? {
            None => {
                println!(
                    "No previous report found at {} — writing initial report.",
                    out_path.display()
                );
            }
            Some(old_report) => {
                let report_diff = diff_reports(&old_report, &new_report);

                match format {
                    OutputFormat::Json => print_diff_json(&report_diff),
                    _ => print_diff_terminal(&report_diff),
                }

                if fail_on_new && !report_diff.new_expired.is_empty() {
                    exit_code = 1;
                }
            }
        }
    }

    write_report(&new_report, out_path)?;
    Ok(exit_code)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::{Annotation, Status};
    use crate::scanner::ScanResult;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_ann(file: &str, line: usize, tag: &str, date: &str, status: Status) -> Annotation {
        Annotation {
            file: PathBuf::from(file),
            line,
            tag: tag.to_string(),
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            owner: None,
            message: "test".to_string(),
            status,
            blamed_owner: None,
        }
    }

    fn make_scan_result(annotations: Vec<Annotation>) -> ScanResult {
        ScanResult {
            scanned_files: 1,
            skipped_files: 0,
            annotations,
        }
    }

    fn make_report_ann(
        file: &str,
        line: usize,
        tag: &str,
        date: &str,
        status: &str,
    ) -> ReportAnnotation {
        ReportAnnotation {
            file: file.to_string(),
            line,
            tag: tag.to_string(),
            date: date.to_string(),
            owner: None,
            message: "test".to_string(),
            status: status.to_string(),
        }
    }

    fn make_report(
        expired: Vec<ReportAnnotation>,
        expiring_soon: Vec<ReportAnnotation>,
        ok: Vec<ReportAnnotation>,
    ) -> Report {
        let total = expired.len() + expiring_soon.len() + ok.len();
        Report {
            generated_at: "2025-01-01T00:00:00+00:00".to_string(),
            scanned_files: 1,
            total_annotations: total,
            expired,
            expiring_soon,
            ok,
        }
    }

    // ── build_report ──────────────────────────────────────────────────────────

    #[test]
    fn test_build_report_empty() {
        let result = make_scan_result(vec![]);
        let report = build_report(&result, "2025-01-01T00:00:00+00:00");
        assert_eq!(report.total_annotations, 0);
        assert!(report.expired.is_empty());
        assert!(report.expiring_soon.is_empty());
        assert!(report.ok.is_empty());
        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.generated_at, "2025-01-01T00:00:00+00:00");
    }

    #[test]
    fn test_build_report_counts() {
        let annotations = vec![
            make_ann("src/a.rs", 1, "TODO", "2020-01-01", Status::Expired),
            make_ann("src/b.rs", 2, "FIXME", "2020-06-01", Status::Expired),
            make_ann("src/c.rs", 3, "TODO", "2025-06-10", Status::ExpiringSoon),
            make_ann("src/d.rs", 4, "TODO", "2099-01-01", Status::Ok),
        ];
        let result = make_scan_result(annotations);
        let report = build_report(&result, "2025-01-01T00:00:00+00:00");

        assert_eq!(report.expired.len(), 2);
        assert_eq!(report.expiring_soon.len(), 1);
        assert_eq!(report.ok.len(), 1);
        assert_eq!(report.total_annotations, 4);
    }

    // ── write_report / read_report roundtrip ─────────────────────────────────

    #[test]
    fn test_write_and_read_report_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        let expired = vec![make_report_ann(
            "src/a.rs",
            1,
            "TODO",
            "2020-01-01",
            "expired",
        )];
        let report = make_report(expired, vec![], vec![]);

        write_report(&report, &path).unwrap();
        let loaded = read_report(&path).unwrap().unwrap();

        assert_eq!(loaded.generated_at, report.generated_at);
        assert_eq!(loaded.scanned_files, report.scanned_files);
        assert_eq!(loaded.total_annotations, report.total_annotations);
        assert_eq!(loaded.expired.len(), 1);
        assert_eq!(loaded.expired[0], report.expired[0]);
        assert!(loaded.expiring_soon.is_empty());
        assert!(loaded.ok.is_empty());
    }

    #[test]
    fn test_write_report_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("report.json");
        let report = make_report(vec![], vec![], vec![]);
        write_report(&report, &path).unwrap();
        assert!(path.exists());
    }

    // ── read_report edge cases ────────────────────────────────────────────────

    #[test]
    fn test_read_report_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.json");
        let result = read_report(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_report_invalid_json_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, b"this is not json at all!!!").unwrap();
        let result = read_report(&path);
        assert!(result.is_err());
    }

    // ── diff_reports ──────────────────────────────────────────────────────────

    #[test]
    fn test_diff_no_change() {
        let ann = make_report_ann("src/a.rs", 1, "TODO", "2020-01-01", "expired");
        let old = make_report(vec![ann.clone()], vec![], vec![]);
        let new = make_report(vec![ann], vec![], vec![]);
        let diff = diff_reports(&old, &new);
        assert!(diff.new_expired.is_empty());
        assert!(diff.resolved.is_empty());
        assert!(diff.new_annotations.is_empty());
        assert!(diff.snoozed.is_empty());
    }

    #[test]
    fn test_diff_new_expired() {
        // Annotation was ok in old report, now expired in new report.
        let ok_ann = make_report_ann("src/a.rs", 1, "TODO", "2020-01-01", "ok");
        let exp_ann = make_report_ann("src/a.rs", 1, "TODO", "2020-01-01", "expired");

        let old = make_report(vec![], vec![], vec![ok_ann]);
        let new = make_report(vec![exp_ann.clone()], vec![], vec![]);

        let diff = diff_reports(&old, &new);
        assert_eq!(diff.new_expired.len(), 1);
        assert_eq!(diff.new_expired[0], exp_ann);
        assert!(diff.resolved.is_empty());
        assert!(diff.new_annotations.is_empty());
    }

    #[test]
    fn test_diff_new_expired_brand_new() {
        // Annotation did not exist at all before, and it's already expired.
        let exp_ann = make_report_ann("src/new.rs", 5, "FIXME", "2020-06-01", "expired");

        let old = make_report(vec![], vec![], vec![]);
        let new = make_report(vec![exp_ann.clone()], vec![], vec![]);

        let diff = diff_reports(&old, &new);
        // Appears in both new_expired and new_annotations
        assert_eq!(diff.new_expired.len(), 1);
        assert_eq!(diff.new_annotations.len(), 1);
    }

    #[test]
    fn test_diff_resolved() {
        // Annotation was expired in old, gone entirely from new.
        let exp_ann = make_report_ann("src/a.rs", 1, "TODO", "2020-01-01", "expired");

        let old = make_report(vec![exp_ann.clone()], vec![], vec![]);
        let new = make_report(vec![], vec![], vec![]);

        let diff = diff_reports(&old, &new);
        assert_eq!(diff.resolved.len(), 1);
        assert_eq!(diff.resolved[0], exp_ann);
        assert!(diff.new_expired.is_empty());
    }

    #[test]
    fn test_diff_new_annotation() {
        // Annotation present in new but not old (ok status).
        let ok_ann = make_report_ann("src/brand_new.rs", 10, "TODO", "2099-01-01", "ok");

        let old = make_report(vec![], vec![], vec![]);
        let new = make_report(vec![], vec![], vec![ok_ann.clone()]);

        let diff = diff_reports(&old, &new);
        assert_eq!(diff.new_annotations.len(), 1);
        assert_eq!(diff.new_annotations[0], ok_ann);
        assert!(diff.new_expired.is_empty());
        assert!(diff.resolved.is_empty());
    }

    #[test]
    fn test_diff_snoozed() {
        // Same key (file, line, tag), but date changed.
        let old_ann = make_report_ann("src/worker.rs", 88, "TODO", "2025-03-01", "ok");
        let new_ann = make_report_ann("src/worker.rs", 88, "TODO", "2026-03-01", "ok");

        let old = make_report(vec![], vec![], vec![old_ann.clone()]);
        let new = make_report(vec![], vec![], vec![new_ann.clone()]);

        let diff = diff_reports(&old, &new);
        assert_eq!(diff.snoozed.len(), 1);
        let (ref before, ref after) = diff.snoozed[0];
        assert_eq!(before.date, "2025-03-01");
        assert_eq!(after.date, "2026-03-01");
        assert!(diff.new_annotations.is_empty());
    }

    #[test]
    fn test_diff_expiring_soon_to_expired_is_new_expired() {
        let expiring_ann = make_report_ann("src/a.rs", 1, "TODO", "2025-06-10", "expiring_soon");
        let expired_ann = make_report_ann("src/a.rs", 1, "TODO", "2025-06-10", "expired");

        let old = make_report(vec![], vec![expiring_ann], vec![]);
        let new = make_report(vec![expired_ann], vec![], vec![]);

        let diff = diff_reports(&old, &new);
        assert_eq!(diff.new_expired.len(), 1);
        assert!(diff.resolved.is_empty());
        assert!(diff.new_annotations.is_empty());
    }

    // ── print functions (smoke tests) ─────────────────────────────────────────

    fn make_nontrivial_diff() -> ReportDiff {
        ReportDiff {
            new_expired: vec![make_report_ann(
                "src/auth/login.rs",
                42,
                "TODO",
                "2025-01-15",
                "expired",
            )],
            resolved: vec![make_report_ann(
                "src/db/old.sql",
                7,
                "TODO",
                "2020-01-01",
                "expired",
            )],
            new_annotations: vec![make_report_ann(
                "src/api/handler.rs",
                12,
                "TODO",
                "2026-06-01",
                "ok",
            )],
            snoozed: vec![(
                make_report_ann("src/worker.rs", 88, "TODO", "2025-03-01", "ok"),
                make_report_ann("src/worker.rs", 88, "TODO", "2026-03-01", "ok"),
            )],
        }
    }

    #[test]
    fn test_print_diff_terminal_does_not_panic() {
        let diff = make_nontrivial_diff();
        // Should not panic — output goes to stdout, which is fine in tests.
        print_diff_terminal(&diff);
    }

    #[test]
    fn test_print_diff_json_does_not_panic() {
        let diff = make_nontrivial_diff();
        print_diff_json(&diff);
    }

    // ── run_report ────────────────────────────────────────────────────────────

    #[test]
    fn test_run_report_no_previous_no_diff() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("report.json");

        let result = make_scan_result(vec![]);
        let code = run_report(
            &result,
            &out_path,
            false, // diff
            false, // fail_on_new
            &OutputFormat::Terminal,
            "2025-01-01T00:00:00+00:00",
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(out_path.exists());

        // Report was written correctly.
        let loaded = read_report(&out_path).unwrap().unwrap();
        assert_eq!(loaded.total_annotations, 0);
    }

    #[test]
    fn test_run_report_diff_no_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("report.json");

        let result = make_scan_result(vec![]);
        let code = run_report(
            &result,
            &out_path,
            true,  // diff = true, but no previous file
            false, // fail_on_new
            &OutputFormat::Terminal,
            "2025-01-01T00:00:00+00:00",
        )
        .unwrap();

        // Should print note and exit 0.
        assert_eq!(code, 0);
        // Should still write the file.
        assert!(out_path.exists());
    }

    #[test]
    fn test_run_report_fail_on_new_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("report.json");

        // Write an initial report with no expired annotations.
        let old_report = make_report(vec![], vec![], vec![]);
        write_report(&old_report, &out_path).unwrap();

        // New scan finds an expired annotation.
        let annotations = vec![make_ann(
            "src/a.rs",
            1,
            "TODO",
            "2020-01-01",
            Status::Expired,
        )];
        let result = make_scan_result(annotations);

        let code = run_report(
            &result,
            &out_path,
            true, // diff
            true, // fail_on_new
            &OutputFormat::Terminal,
            "2025-06-01T00:00:00+00:00",
        )
        .unwrap();

        assert_eq!(code, 1);
    }

    #[test]
    fn test_run_report_fail_on_new_exits_zero_when_no_new_expired() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("report.json");

        // Write an initial report with the same expired annotation.
        let expired = vec![make_report_ann(
            "src/a.rs",
            1,
            "TODO",
            "2020-01-01",
            "expired",
        )];
        let old_report = make_report(expired, vec![], vec![]);
        write_report(&old_report, &out_path).unwrap();

        // New scan finds the same expired annotation — not "new".
        let annotations = vec![make_ann(
            "src/a.rs",
            1,
            "TODO",
            "2020-01-01",
            Status::Expired,
        )];
        let result = make_scan_result(annotations);

        let code = run_report(
            &result,
            &out_path,
            true, // diff
            true, // fail_on_new
            &OutputFormat::Terminal,
            "2025-06-01T00:00:00+00:00",
        )
        .unwrap();

        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_report_json_format_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("report.json");

        // Seed a previous report.
        let old_report = make_report(vec![], vec![], vec![]);
        write_report(&old_report, &out_path).unwrap();

        let annotations = vec![make_ann("src/b.rs", 99, "FIXME", "2099-12-01", Status::Ok)];
        let result = make_scan_result(annotations);

        let code = run_report(
            &result,
            &out_path,
            true,
            false,
            &OutputFormat::Json,
            "2025-06-01T00:00:00+00:00",
        )
        .unwrap();

        assert_eq!(code, 0);
    }
}
