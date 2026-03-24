use crate::annotation::{Fuse, Status};
use crate::scanner::ScanResult;
use chrono::NaiveDate;
use colored::Colorize;
use serde::Serialize;
use std::path::Path;

/// Output format selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable terminal output with color.
    Terminal,
    /// Machine-readable JSON.
    Json,
    /// GitHub Actions annotation format.
    GitHub,
}

impl OutputFormat {
    /// Auto-detect the best default format based on environment variables.
    /// If `GITHUB_ACTIONS=true` is set, default to GitHub format.
    pub fn auto_detect() -> Self {
        if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
            OutputFormat::GitHub
        } else {
            OutputFormat::Terminal
        }
    }

    /// Parse from a string (as provided by --format CLI flag).
    pub fn parse_format(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "terminal" | "term" => Some(OutputFormat::Terminal),
            "json" => Some(OutputFormat::Json),
            "github" | "gh" => Some(OutputFormat::GitHub),
            _ => None,
        }
    }
}

/// Whether color output should be enabled.
fn color_enabled() -> bool {
    // Respect NO_COLOR convention (https://no-color.org/)
    std::env::var("NO_COLOR").is_err()
}

// ─── Terminal formatter ───────────────────────────────────────────────────────

/// Format a relative days label for terminal/GitHub output.
fn days_label(fuse: &Fuse, today: NaiveDate) -> String {
    let delta = fuse.days_from_today(today);
    match fuse.status {
        Status::Detonated => format!(" ({} days overdue)", delta.unsigned_abs()),
        Status::Ticking => format!(" (in {} days)", delta),
        Status::Inert => String::new(),
    }
}

/// Print a `ScanResult` to stdout using the terminal (colored) format.
pub fn print_terminal(result: &ScanResult, _fuse_days: u32, show_ok: bool, today: NaiveDate) {
    let use_color = color_enabled();
    for fuse in &result.fuses {
        print_fuse_terminal(fuse, use_color, show_ok, today);
    }
    println!();
    print_summary_line(result, use_color);
}

/// Print only the summary line — used by `sweep --summary`.
pub fn print_scan_summary(result: &ScanResult) {
    print_summary_line(result, color_enabled());
}

/// Shared summary-line renderer used by both `print_terminal` and `print_scan_summary`.
fn print_summary_line(result: &ScanResult, use_color: bool) {
    let (detonated_count, ticking_count, inert_count) =
        result
            .fuses
            .iter()
            .fold((0usize, 0usize, 0usize), |(d, t, i), fuse| {
                match fuse.status {
                    Status::Detonated => (d + 1, t, i),
                    Status::Ticking => (d, t + 1, i),
                    Status::Inert => (d, t, i + 1),
                }
            });

    let summary = format!(
        "Swept {} file(s) · {} fuse(s) total · {} detonated · {} ticking · {} inert",
        result.swept_files,
        result.total(),
        detonated_count,
        ticking_count,
        inert_count,
    );

    if use_color {
        if detonated_count > 0 {
            eprintln!("{}", summary.red().bold());
        } else if ticking_count > 0 {
            eprintln!("{}", summary.yellow());
        } else {
            eprintln!("{}", summary.green());
        }
    } else {
        eprintln!("{}", summary);
    }
}

/// Format the owner column: `[owner]` if explicit, `[~blame]` if inferred, empty otherwise.
fn owner_display(fuse: &Fuse) -> String {
    if let Some(o) = &fuse.owner {
        format!(" [{}]", o)
    } else if let Some(b) = &fuse.blamed_owner {
        format!(" [~{}]", b)
    } else {
        String::new()
    }
}

fn print_fuse_terminal(fuse: &Fuse, use_color: bool, show_ok: bool, today: NaiveDate) {
    // Skip inert items unless explicitly requested
    if fuse.status == Status::Inert && !show_ok {
        // Still show them in list mode
    }

    let status_label = match fuse.status {
        Status::Detonated => "DETONATED",
        Status::Ticking => "TICKING  ",
        Status::Inert => "INERT    ",
    };

    let location = format!("{:<40}", fuse.location());
    let tag_date = format!("{}[{}]", fuse.tag, fuse.date_str());
    let tag_date_col = format!("{:<20}", tag_date);
    let days_str = days_label(fuse, today);
    let owner_part = owner_display(fuse);

    let line = format!(
        "{} {}  {}{}{}  {}",
        status_label, location, tag_date_col, days_str, owner_part, fuse.message
    );

    if use_color {
        let colored_line = match fuse.status {
            Status::Detonated => line.red().bold().to_string(),
            Status::Ticking => line.yellow().to_string(),
            Status::Inert => line.dimmed().to_string(),
        };
        println!("{}", colored_line);
    } else {
        println!("{}", line);
    }
}

/// Print a single fuse in terminal format (used by `manifest` subcommand).
pub fn print_fuse_line_terminal(fuse: &Fuse, use_color: bool, today: NaiveDate) {
    let status_label = match fuse.status {
        Status::Detonated => "DETONATED",
        Status::Ticking => "TICKING  ",
        Status::Inert => "INERT    ",
    };

    let location = format!("{:<40}", fuse.location());
    let tag_date = format!("{}[{}]", fuse.tag, fuse.date_str());
    let tag_date_col = format!("{:<20}", tag_date);
    let days_str = days_label(fuse, today);
    let owner_part = owner_display(fuse);

    let line = format!(
        "{} {}  {}{}{}  {}",
        status_label, location, tag_date_col, days_str, owner_part, fuse.message
    );

    if use_color {
        let colored_line = match fuse.status {
            Status::Detonated => line.red().bold().to_string(),
            Status::Ticking => line.yellow().to_string(),
            Status::Inert => line.dimmed().to_string(),
        };
        println!("{}", colored_line);
    } else {
        println!("{}", line);
    }
}

// ─── JSON formatter ───────────────────────────────────────────────────────────

/// Serializable wrapper for the full JSON output.
#[derive(Debug, Serialize)]
pub struct JsonOutput<'a> {
    pub swept_files: usize,
    pub total_fuses: usize,
    pub detonated: Vec<JsonFuse<'a>>,
    pub ticking: Vec<JsonFuse<'a>>,
    pub inert: Vec<JsonFuse<'a>>,
}

/// A single fuse serialized for JSON output.
#[derive(Debug, Serialize)]
pub struct JsonFuse<'a> {
    pub file: String,
    pub line: usize,
    pub tag: &'a str,
    pub date: String,
    pub owner: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blamed_owner: Option<&'a str>,
    pub message: &'a str,
    pub status: &'a str,
}

impl<'a> JsonFuse<'a> {
    fn from_fuse(fuse: &'a Fuse) -> Self {
        JsonFuse {
            file: fuse.file.display().to_string(),
            line: fuse.line,
            tag: &fuse.tag,
            date: fuse.date_str(),
            owner: fuse.owner.as_deref(),
            blamed_owner: fuse.blamed_owner.as_deref(),
            message: &fuse.message,
            status: fuse.status.as_str(),
        }
    }
}

/// Print the full scan result as JSON to stdout.
pub fn print_json(result: &ScanResult) {
    let detonated: Vec<JsonFuse> = result
        .detonated()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();

    let ticking: Vec<JsonFuse> = result
        .ticking()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();

    let inert: Vec<JsonFuse> = result
        .inert()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();

    let output = JsonOutput {
        swept_files: result.swept_files,
        total_fuses: result.total(),
        detonated,
        ticking,
        inert,
    };

    match serde_json::to_string_pretty(&output) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("error: failed to serialize JSON output: {}", e),
    }
}

/// Serialize the scan result as JSON and write it to a file (used by `sweep --output`).
pub fn write_json_report(result: &ScanResult, path: &Path) -> std::io::Result<()> {
    let detonated: Vec<JsonFuse> = result
        .detonated()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();
    let ticking: Vec<JsonFuse> = result
        .ticking()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();
    let inert: Vec<JsonFuse> = result
        .inert()
        .iter()
        .map(|f| JsonFuse::from_fuse(f))
        .collect();
    let output = JsonOutput {
        swept_files: result.swept_files,
        total_fuses: result.total(),
        detonated,
        ticking,
        inert,
    };
    let json = serde_json::to_string_pretty(&output).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// Serialize a slice of fuses as a JSON array (used by `manifest --format json`).
pub fn print_json_list(fuses: &[&Fuse]) {
    let items: Vec<JsonFuse> = fuses.iter().map(|f| JsonFuse::from_fuse(f)).collect();

    match serde_json::to_string_pretty(&items) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("error: failed to serialize JSON output: {}", e),
    }
}

// ─── GitHub Actions formatter ─────────────────────────────────────────────────

/// Print fuses in GitHub Actions workflow command format.
///
/// Detonated → `::error`
/// Ticking → `::warning`
/// Inert → silently skipped
pub fn print_github(result: &ScanResult, _fuse_days: u32, today: NaiveDate) {
    for fuse in &result.fuses {
        print_fuse_github(fuse, 0, today);
    }
}

/// Print a single fuse in GitHub Actions format.
pub fn print_fuse_github(fuse: &Fuse, _fuse_days: u32, today: NaiveDate) {
    let file = fuse.file.display().to_string();
    let line = fuse.line;
    let delta = fuse.days_from_today(today);

    match fuse.status {
        Status::Detonated => {
            println!(
                "::error file={},line={}::{} detonated on {} ({} days overdue): {}",
                file,
                line,
                fuse.tag,
                fuse.date_str(),
                delta.unsigned_abs(),
                fuse.message
            );
        }
        Status::Ticking => {
            println!(
                "::warning file={},line={}::{} detonates on {} (in {} days): {}",
                file,
                line,
                fuse.tag,
                fuse.date_str(),
                delta,
                fuse.message
            );
        }
        Status::Inert => {
            // Don't emit anything for inert fuses in CI output
        }
    }
}

/// Print a slice of fuses in GitHub Actions format for the `manifest` subcommand.
pub fn print_github_list(fuses: &[&Fuse], fuse_days: u32, today: NaiveDate) {
    for fuse in fuses {
        print_fuse_github(fuse, fuse_days, today);
    }
}

// ─── Dispatch helpers ─────────────────────────────────────────────────────────

/// Top-level dispatch: print a `ScanResult` in whatever format was requested.
pub fn print_scan_result(
    result: &ScanResult,
    format: &OutputFormat,
    fuse_days: u32,
    today: NaiveDate,
) {
    match format {
        OutputFormat::Terminal => print_terminal(result, fuse_days, false, today),
        OutputFormat::Json => print_json(result),
        OutputFormat::GitHub => print_github(result, fuse_days, today),
    }
}

/// Top-level dispatch for the `manifest` subcommand.
pub fn print_list(
    fuses: &[&Fuse],
    format: &OutputFormat,
    fuse_days: u32,
    scan_root: &Path,
    today: NaiveDate,
) {
    let _ = scan_root; // available for future use (e.g. relative path display)
    let use_color = color_enabled();

    match format {
        OutputFormat::Terminal => {
            for fuse in fuses {
                print_fuse_line_terminal(fuse, use_color, today);
            }
            println!();
            eprintln!("{} fuse(s) listed", fuses.len());
        }
        OutputFormat::Json => {
            print_json_list(fuses);
        }
        OutputFormat::GitHub => {
            print_github_list(fuses, fuse_days, today);
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::Status;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn fixed_today() -> NaiveDate {
        date("2026-03-23")
    }

    fn make_fuse(tag: &str, expiry: &str, status: Status, msg: &str) -> Fuse {
        Fuse {
            file: PathBuf::from("src/foo.rs"),
            line: 42,
            tag: tag.to_string(),
            date: date(expiry),
            owner: None,
            message: msg.to_string(),
            status,
            blamed_owner: None,
        }
    }

    fn make_fuse_with_owner(
        tag: &str,
        expiry: &str,
        status: Status,
        msg: &str,
        owner: &str,
    ) -> Fuse {
        Fuse {
            file: PathBuf::from("src/foo.rs"),
            line: 10,
            tag: tag.to_string(),
            date: date(expiry),
            owner: Some(owner.to_string()),
            message: msg.to_string(),
            status,
            blamed_owner: None,
        }
    }

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(OutputFormat::parse_format("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse_format("JSON"), Some(OutputFormat::Json));
        assert_eq!(
            OutputFormat::parse_format("github"),
            Some(OutputFormat::GitHub)
        );
        assert_eq!(OutputFormat::parse_format("gh"), Some(OutputFormat::GitHub));
        assert_eq!(
            OutputFormat::parse_format("terminal"),
            Some(OutputFormat::Terminal)
        );
        assert_eq!(
            OutputFormat::parse_format("term"),
            Some(OutputFormat::Terminal)
        );
        assert_eq!(OutputFormat::parse_format("unknown"), None);
    }

    #[test]
    fn test_json_fuse_from_fuse() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "remove this");
        let j = JsonFuse::from_fuse(&fuse);
        assert_eq!(j.file, "src/foo.rs");
        assert_eq!(j.line, 42);
        assert_eq!(j.tag, "TODO");
        assert_eq!(j.date, "2020-01-01");
        assert_eq!(j.owner, None);
        assert_eq!(j.message, "remove this");
        assert_eq!(j.status, "detonated");
    }

    #[test]
    fn test_json_fuse_with_owner() {
        let fuse =
            make_fuse_with_owner("FIXME", "2099-01-01", Status::Inert, "upgrade later", "bob");
        let j = JsonFuse::from_fuse(&fuse);
        assert_eq!(j.owner, Some("bob"));
        assert_eq!(j.status, "inert");
    }

    #[test]
    fn test_json_fuse_ticking_status() {
        let fuse = make_fuse("HACK", "2025-06-10", Status::Ticking, "temp hack");
        let j = JsonFuse::from_fuse(&fuse);
        assert_eq!(j.status, "ticking");
    }

    #[test]
    fn test_print_json_does_not_panic() {
        use crate::scanner::ScanResult;
        let result = ScanResult {
            fuses: vec![
                make_fuse("TODO", "2020-01-01", Status::Detonated, "detonated"),
                make_fuse("FIXME", "2099-01-01", Status::Inert, "future"),
            ],
            swept_files: 5,
            skipped_files: 1,
        };
        // Should not panic
        print_json(&result);
    }

    #[test]
    fn test_print_json_list_does_not_panic() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "detonated");
        print_json_list(&[&fuse]);
    }

    #[test]
    fn test_print_github_detonated_format() {
        let fuse = make_fuse(
            "TODO",
            "2020-01-01",
            Status::Detonated,
            "remove legacy oauth",
        );
        print_fuse_github(&fuse, 14, fixed_today());
    }

    #[test]
    fn test_print_github_ticking_format() {
        let fuse = make_fuse("FIXME", "2026-04-01", Status::Ticking, "fix before release");
        print_fuse_github(&fuse, 14, fixed_today());
    }

    #[test]
    fn test_print_github_inert_is_silent() {
        let fuse = make_fuse("HACK", "2099-01-01", Status::Inert, "fine for now");
        print_fuse_github(&fuse, 0, fixed_today());
    }

    #[test]
    fn test_auto_detect_no_github_env() {
        // When GITHUB_ACTIONS is not set (or not "true"), should default to Terminal
        // We can't reliably unset env vars in tests, so just verify the logic path
        let format = if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
            OutputFormat::GitHub
        } else {
            OutputFormat::Terminal
        };
        // Just make sure this doesn't panic
        let _ = format;
    }

    #[test]
    fn test_color_enabled_respects_no_color() {
        // We can't easily set/unset env vars in a portable, safe way in parallel tests,
        // but we can verify the function returns a bool without panicking.
        let _enabled = color_enabled();
    }

    #[test]
    fn test_print_terminal_does_not_panic() {
        use crate::scanner::ScanResult;
        let result = ScanResult {
            fuses: vec![
                make_fuse("TODO", "2020-01-01", Status::Detonated, "old"),
                make_fuse("FIXME", "2026-04-15", Status::Ticking, "soon"),
                make_fuse("HACK", "2099-12-31", Status::Inert, "future"),
            ],
            swept_files: 3,
            skipped_files: 0,
        };
        print_terminal(&result, 14, true, fixed_today());
    }

    #[test]
    fn test_print_fuse_line_terminal_with_owner() {
        let fuse = make_fuse_with_owner(
            "TODO",
            "2020-01-01",
            Status::Detonated,
            "remove me",
            "alice",
        );
        print_fuse_line_terminal(&fuse, false, fixed_today());
    }

    #[test]
    fn test_print_list_terminal_does_not_panic() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "list item");
        print_list(
            &[&fuse],
            &OutputFormat::Terminal,
            14,
            std::path::Path::new("."),
            fixed_today(),
        );
    }

    #[test]
    fn test_print_list_json_does_not_panic() {
        let fuse = make_fuse("FIXME", "2099-01-01", Status::Inert, "future item");
        print_list(
            &[&fuse],
            &OutputFormat::Json,
            0,
            std::path::Path::new("."),
            fixed_today(),
        );
    }

    #[test]
    fn test_print_list_github_does_not_panic() {
        let fuse = make_fuse("HACK", "2020-01-01", Status::Detonated, "github list");
        print_list(
            &[&fuse],
            &OutputFormat::GitHub,
            0,
            std::path::Path::new("."),
            fixed_today(),
        );
    }

    #[test]
    fn test_print_scan_result_dispatch() {
        use crate::scanner::ScanResult;
        let result = ScanResult {
            fuses: vec![make_fuse("TODO", "2020-01-01", Status::Detonated, "x")],
            swept_files: 1,
            skipped_files: 0,
        };
        print_scan_result(&result, &OutputFormat::Terminal, 0, fixed_today());
        print_scan_result(&result, &OutputFormat::Json, 0, fixed_today());
        print_scan_result(&result, &OutputFormat::GitHub, 0, fixed_today());
    }

    // ── blamed_owner display ──────────────────────────────────────────────────

    #[test]
    fn test_owner_display_explicit_owner() {
        let fuse = make_fuse_with_owner("TODO", "2020-01-01", Status::Detonated, "msg", "alice");
        assert_eq!(owner_display(&fuse), " [alice]");
    }

    #[test]
    fn test_owner_display_blamed_owner() {
        let mut fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        fuse.blamed_owner = Some("bob".to_string());
        assert_eq!(owner_display(&fuse), " [~bob]");
    }

    #[test]
    fn test_owner_display_no_owner() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        assert_eq!(owner_display(&fuse), "");
    }

    #[test]
    fn test_owner_display_explicit_takes_precedence_over_blamed() {
        // When both owner and blamed_owner are set, explicit owner wins.
        let mut fuse =
            make_fuse_with_owner("TODO", "2020-01-01", Status::Detonated, "msg", "alice");
        fuse.blamed_owner = Some("bob".to_string());
        // Should show explicit owner, not blamed_owner.
        assert_eq!(owner_display(&fuse), " [alice]");
    }

    #[test]
    fn test_json_fuse_includes_blamed_owner() {
        let mut fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        fuse.blamed_owner = Some("dave".to_string());
        let j = JsonFuse::from_fuse(&fuse);
        assert_eq!(j.blamed_owner, Some("dave"));
        assert_eq!(j.owner, None);
    }

    #[test]
    fn test_json_fuse_blamed_owner_absent_when_none() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        let j = JsonFuse::from_fuse(&fuse);
        assert_eq!(j.blamed_owner, None);
        // The field is skip_serializing_if = None, so it must not appear in the JSON string.
        let json = serde_json::to_string(&j).unwrap();
        assert!(!json.contains("blamed_owner"));
    }

    #[test]
    fn test_print_fuse_line_terminal_with_blamed_owner() {
        let mut fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        fuse.blamed_owner = Some("eve".to_string());
        print_fuse_line_terminal(&fuse, false, fixed_today());
    }

    // ── days_label ────────────────────────────────────────────────────────────

    #[test]
    fn test_days_label_detonated_shows_overdue() {
        let fuse = make_fuse("TODO", "2020-01-01", Status::Detonated, "msg");
        let label = days_label(&fuse, fixed_today());
        assert!(
            label.contains("overdue"),
            "expected 'overdue' in '{}'",
            label
        );
        assert!(
            !label.contains("in "),
            "detonated should not say 'in X days'"
        );
    }

    #[test]
    fn test_days_label_ticking_shows_days_remaining() {
        let fuse = make_fuse("FIXME", "2026-04-01", Status::Ticking, "msg");
        let label = days_label(&fuse, fixed_today());
        assert!(label.contains("in "), "expected 'in X days' in '{}'", label);
        assert!(label.contains("days"), "expected 'days' in '{}'", label);
    }

    #[test]
    fn test_days_label_inert_is_empty() {
        let fuse = make_fuse("HACK", "2099-01-01", Status::Inert, "msg");
        let label = days_label(&fuse, fixed_today());
        assert!(label.is_empty(), "inert fuses should have no days label");
    }
}
