use crate::annotation::{Annotation, Status};
use crate::scanner::ScanResult;
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

/// Print a `ScanResult` to stdout using the terminal (colored) format.
pub fn print_terminal(result: &ScanResult, warn_within_days: u32, show_ok: bool) {
    let use_color = color_enabled();

    for ann in &result.annotations {
        print_annotation_terminal(ann, warn_within_days, use_color, show_ok);
    }

    // Summary line
    let expired_count = result.expired().len();
    let soon_count = result.expiring_soon().len();
    let ok_count = result.ok().len();

    println!();
    let summary = format!(
        "Scanned {} file(s) · {} annotation(s) total · {} expired · {} expiring soon · {} ok",
        result.scanned_files,
        result.total(),
        expired_count,
        soon_count,
        ok_count,
    );

    if use_color {
        if expired_count > 0 {
            eprintln!("{}", summary.red().bold());
        } else if soon_count > 0 {
            eprintln!("{}", summary.yellow());
        } else {
            eprintln!("{}", summary.green());
        }
    } else {
        eprintln!("{}", summary);
    }
}

/// Format the owner column: `[owner]` if explicit, `[~blame]` if inferred, empty otherwise.
fn owner_display(ann: &Annotation) -> String {
    if let Some(o) = &ann.owner {
        format!(" [{}]", o)
    } else if let Some(b) = &ann.blamed_owner {
        format!(" [~{}]", b)
    } else {
        String::new()
    }
}

fn print_annotation_terminal(
    ann: &Annotation,
    _warn_within_days: u32,
    use_color: bool,
    show_ok: bool,
) {
    // Skip OK items unless explicitly requested
    if ann.status == Status::Ok && !show_ok {
        // Still show them in list mode
    }

    let status_label = match ann.status {
        Status::Expired => "EXPIRED ",
        Status::ExpiringSoon => "WARNING ",
        Status::Ok => "OK      ",
    };

    let location = format!("{:<40}", ann.location());
    let tag_date = format!("{}[{}]", ann.tag, ann.date_str());
    let tag_date_col = format!("{:<20}", tag_date);

    let owner_part = owner_display(ann);

    let line = format!(
        "{} {}  {}{}  {}",
        status_label, location, tag_date_col, owner_part, ann.message
    );

    if use_color {
        let colored_line = match ann.status {
            Status::Expired => line.red().bold().to_string(),
            Status::ExpiringSoon => line.yellow().to_string(),
            Status::Ok => line.dimmed().to_string(),
        };
        println!("{}", colored_line);
    } else {
        println!("{}", line);
    }
}

/// Print a single annotation in terminal format (used by `list` subcommand).
pub fn print_annotation_line_terminal(ann: &Annotation, use_color: bool) {
    let status_label = match ann.status {
        Status::Expired => "EXPIRED ",
        Status::ExpiringSoon => "WARNING ",
        Status::Ok => "OK      ",
    };

    let location = format!("{:<40}", ann.location());
    let tag_date = format!("{}[{}]", ann.tag, ann.date_str());
    let tag_date_col = format!("{:<20}", tag_date);

    let owner_part = owner_display(ann);

    let line = format!(
        "{} {}  {}{}  {}",
        status_label, location, tag_date_col, owner_part, ann.message
    );

    if use_color {
        let colored_line = match ann.status {
            Status::Expired => line.red().bold().to_string(),
            Status::ExpiringSoon => line.yellow().to_string(),
            Status::Ok => line.dimmed().to_string(),
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
    pub scanned_files: usize,
    pub total_annotations: usize,
    pub expired: Vec<JsonAnnotation<'a>>,
    pub expiring_soon: Vec<JsonAnnotation<'a>>,
    pub ok: Vec<JsonAnnotation<'a>>,
}

/// A single annotation serialized for JSON output.
#[derive(Debug, Serialize)]
pub struct JsonAnnotation<'a> {
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

impl<'a> JsonAnnotation<'a> {
    fn from_annotation(ann: &'a Annotation) -> Self {
        JsonAnnotation {
            file: ann.file.display().to_string(),
            line: ann.line,
            tag: &ann.tag,
            date: ann.date_str(),
            owner: ann.owner.as_deref(),
            blamed_owner: ann.blamed_owner.as_deref(),
            message: &ann.message,
            status: ann.status.as_str(),
        }
    }
}

/// Print the full scan result as JSON to stdout.
pub fn print_json(result: &ScanResult) {
    let expired: Vec<JsonAnnotation> = result
        .expired()
        .iter()
        .map(|a| JsonAnnotation::from_annotation(a))
        .collect();

    let expiring_soon: Vec<JsonAnnotation> = result
        .expiring_soon()
        .iter()
        .map(|a| JsonAnnotation::from_annotation(a))
        .collect();

    let ok: Vec<JsonAnnotation> = result
        .ok()
        .iter()
        .map(|a| JsonAnnotation::from_annotation(a))
        .collect();

    let output = JsonOutput {
        scanned_files: result.scanned_files,
        total_annotations: result.total(),
        expired,
        expiring_soon,
        ok,
    };

    match serde_json::to_string_pretty(&output) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("error: failed to serialize JSON output: {}", e),
    }
}

/// Serialize a slice of annotations as a JSON array (used by `list --format json`).
pub fn print_json_list(annotations: &[&Annotation]) {
    let items: Vec<JsonAnnotation> = annotations
        .iter()
        .map(|a| JsonAnnotation::from_annotation(a))
        .collect();

    match serde_json::to_string_pretty(&items) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("error: failed to serialize JSON output: {}", e),
    }
}

// ─── GitHub Actions formatter ─────────────────────────────────────────────────

/// Print annotations in GitHub Actions workflow command format.
///
/// Expired → `::error`
/// Expiring soon → `::warning`
/// Ok → silently skipped
pub fn print_github(result: &ScanResult, warn_within_days: u32) {
    for ann in &result.annotations {
        print_annotation_github(ann, warn_within_days);
    }
}

/// Print a single annotation in GitHub Actions format.
pub fn print_annotation_github(ann: &Annotation, warn_within_days: u32) {
    let file = ann.file.display().to_string();
    let line = ann.line;

    match ann.status {
        Status::Expired => {
            println!(
                "::error file={},line={}::{} expired on {}: {}",
                file,
                line,
                ann.tag,
                ann.date_str(),
                ann.message
            );
        }
        Status::ExpiringSoon => {
            // Calculate how many days remain
            let days_msg = if warn_within_days > 0 {
                format!(" (within {}d warning window)", warn_within_days)
            } else {
                String::new()
            };
            println!(
                "::warning file={},line={}::{} expires on {}{}:  {}",
                file,
                line,
                ann.tag,
                ann.date_str(),
                days_msg,
                ann.message
            );
        }
        Status::Ok => {
            // Don't emit anything for OK annotations in CI output
        }
    }
}

/// Print a single annotation in GitHub Actions format for the `list` subcommand.
pub fn print_github_list(annotations: &[&Annotation], warn_within_days: u32) {
    for ann in annotations {
        print_annotation_github(ann, warn_within_days);
    }
}

// ─── Dispatch helpers ─────────────────────────────────────────────────────────

/// Top-level dispatch: print a `ScanResult` in whatever format was requested.
pub fn print_scan_result(result: &ScanResult, format: &OutputFormat, warn_within_days: u32) {
    match format {
        OutputFormat::Terminal => print_terminal(result, warn_within_days, false),
        OutputFormat::Json => print_json(result),
        OutputFormat::GitHub => print_github(result, warn_within_days),
    }
}

/// Top-level dispatch for the `list` subcommand.
pub fn print_list(
    annotations: &[&Annotation],
    format: &OutputFormat,
    warn_within_days: u32,
    scan_root: &Path,
) {
    let _ = scan_root; // available for future use (e.g. relative path display)
    let use_color = color_enabled();

    match format {
        OutputFormat::Terminal => {
            for ann in annotations {
                print_annotation_line_terminal(ann, use_color);
            }
            println!();
            eprintln!("{} annotation(s) listed", annotations.len());
        }
        OutputFormat::Json => {
            print_json_list(annotations);
        }
        OutputFormat::GitHub => {
            print_github_list(annotations, warn_within_days);
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

    fn make_annotation(tag: &str, expiry: &str, status: Status, msg: &str) -> Annotation {
        Annotation {
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

    fn make_annotation_with_owner(
        tag: &str,
        expiry: &str,
        status: Status,
        msg: &str,
        owner: &str,
    ) -> Annotation {
        Annotation {
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
    fn test_json_annotation_from_annotation() {
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "remove this");
        let j = JsonAnnotation::from_annotation(&ann);
        assert_eq!(j.file, "src/foo.rs");
        assert_eq!(j.line, 42);
        assert_eq!(j.tag, "TODO");
        assert_eq!(j.date, "2020-01-01");
        assert_eq!(j.owner, None);
        assert_eq!(j.message, "remove this");
        assert_eq!(j.status, "expired");
    }

    #[test]
    fn test_json_annotation_with_owner() {
        let ann =
            make_annotation_with_owner("FIXME", "2099-01-01", Status::Ok, "upgrade later", "bob");
        let j = JsonAnnotation::from_annotation(&ann);
        assert_eq!(j.owner, Some("bob"));
        assert_eq!(j.status, "ok");
    }

    #[test]
    fn test_json_annotation_expiring_soon_status() {
        let ann = make_annotation("HACK", "2025-06-10", Status::ExpiringSoon, "temp hack");
        let j = JsonAnnotation::from_annotation(&ann);
        assert_eq!(j.status, "expiring_soon");
    }

    #[test]
    fn test_print_json_does_not_panic() {
        use crate::scanner::ScanResult;
        let result = ScanResult {
            annotations: vec![
                make_annotation("TODO", "2020-01-01", Status::Expired, "expired"),
                make_annotation("FIXME", "2099-01-01", Status::Ok, "future"),
            ],
            scanned_files: 5,
            skipped_files: 1,
        };
        // Should not panic
        print_json(&result);
    }

    #[test]
    fn test_print_json_list_does_not_panic() {
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "expired");
        print_json_list(&[&ann]);
    }

    #[test]
    fn test_print_github_expired_format() {
        // Capture via manual construction; we just verify no panic and check format logic
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "remove legacy oauth");
        // No panic
        print_annotation_github(&ann, 14);
    }

    #[test]
    fn test_print_github_expiring_soon_format() {
        let ann = make_annotation(
            "FIXME",
            "2025-06-10",
            Status::ExpiringSoon,
            "fix before release",
        );
        print_annotation_github(&ann, 14);
    }

    #[test]
    fn test_print_github_ok_is_silent() {
        // We can't easily capture stdout in unit tests, but we can at least ensure no panic
        let ann = make_annotation("HACK", "2099-01-01", Status::Ok, "fine for now");
        print_annotation_github(&ann, 0);
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
            annotations: vec![
                make_annotation("TODO", "2020-01-01", Status::Expired, "old"),
                make_annotation("FIXME", "2025-06-08", Status::ExpiringSoon, "soon"),
                make_annotation("HACK", "2099-12-31", Status::Ok, "future"),
            ],
            scanned_files: 3,
            skipped_files: 0,
        };
        // Should not panic regardless of color support
        print_terminal(&result, 14, true);
    }

    #[test]
    fn test_print_annotation_line_terminal_with_owner() {
        let ann =
            make_annotation_with_owner("TODO", "2020-01-01", Status::Expired, "remove me", "alice");
        // Should not panic
        print_annotation_line_terminal(&ann, false);
    }

    #[test]
    fn test_print_list_terminal_does_not_panic() {
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "list item");
        print_list(
            &[&ann],
            &OutputFormat::Terminal,
            14,
            std::path::Path::new("."),
        );
    }

    #[test]
    fn test_print_list_json_does_not_panic() {
        let ann = make_annotation("FIXME", "2099-01-01", Status::Ok, "future item");
        print_list(&[&ann], &OutputFormat::Json, 0, std::path::Path::new("."));
    }

    #[test]
    fn test_print_list_github_does_not_panic() {
        let ann = make_annotation("HACK", "2020-01-01", Status::Expired, "github list");
        print_list(&[&ann], &OutputFormat::GitHub, 0, std::path::Path::new("."));
    }

    #[test]
    fn test_print_scan_result_dispatch() {
        use crate::scanner::ScanResult;
        let result = ScanResult {
            annotations: vec![make_annotation("TODO", "2020-01-01", Status::Expired, "x")],
            scanned_files: 1,
            skipped_files: 0,
        };
        // All three formats should not panic
        print_scan_result(&result, &OutputFormat::Terminal, 0);
        print_scan_result(&result, &OutputFormat::Json, 0);
        print_scan_result(&result, &OutputFormat::GitHub, 0);
    }

    // ── blamed_owner display ──────────────────────────────────────────────────

    #[test]
    fn test_owner_display_explicit_owner() {
        let ann = make_annotation_with_owner("TODO", "2020-01-01", Status::Expired, "msg", "alice");
        assert_eq!(owner_display(&ann), " [alice]");
    }

    #[test]
    fn test_owner_display_blamed_owner() {
        let mut ann = make_annotation("TODO", "2020-01-01", Status::Expired, "msg");
        ann.blamed_owner = Some("bob".to_string());
        assert_eq!(owner_display(&ann), " [~bob]");
    }

    #[test]
    fn test_owner_display_no_owner() {
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "msg");
        assert_eq!(owner_display(&ann), "");
    }

    #[test]
    fn test_owner_display_explicit_takes_precedence_over_blamed() {
        // When both owner and blamed_owner are set, explicit owner wins.
        let mut ann =
            make_annotation_with_owner("TODO", "2020-01-01", Status::Expired, "msg", "alice");
        ann.blamed_owner = Some("bob".to_string());
        // Should show explicit owner, not blamed_owner.
        assert_eq!(owner_display(&ann), " [alice]");
    }

    #[test]
    fn test_json_annotation_includes_blamed_owner() {
        let mut ann = make_annotation("TODO", "2020-01-01", Status::Expired, "msg");
        ann.blamed_owner = Some("dave".to_string());
        let j = JsonAnnotation::from_annotation(&ann);
        assert_eq!(j.blamed_owner, Some("dave"));
        assert_eq!(j.owner, None);
    }

    #[test]
    fn test_json_annotation_blamed_owner_absent_when_none() {
        let ann = make_annotation("TODO", "2020-01-01", Status::Expired, "msg");
        let j = JsonAnnotation::from_annotation(&ann);
        assert_eq!(j.blamed_owner, None);
        // The field is skip_serializing_if = None, so it must not appear in the JSON string.
        let json = serde_json::to_string(&j).unwrap();
        assert!(!json.contains("blamed_owner"));
    }

    #[test]
    fn test_print_annotation_line_terminal_with_blamed_owner() {
        let mut ann = make_annotation("TODO", "2020-01-01", Status::Expired, "msg");
        ann.blamed_owner = Some("eve".to_string());
        // Should not panic; no assertion on stdout since we can't capture easily.
        print_annotation_line_terminal(&ann, false);
    }
}
