//! Logic for the `timebomb snooze` subcommand.
//!
//! This module implements the core logic for bumping the expiry date of an
//! existing timebomb annotation in-place without manually editing the file.
//!
//! NOTE to cli agent: when wiring up `SnoozeArgs`, define it as:
//!
//! ```ignore
//! /// Arguments for the `snooze` subcommand.
//! #[derive(Debug, clap::Args)]
//! pub struct SnoozeArgs {
//!     /// Target file and line number in the form "path/to/file.rs:42"
//!     #[arg(value_name = "FILE:LINE")]
//!     pub target: String,
//!
//!     /// New expiry date as YYYY-MM-DD (mutually exclusive with --in-days)
//!     #[arg(long, value_name = "DATE", conflicts_with = "in_days")]
//!     pub date: Option<String>,
//!
//!     /// Number of days from today until new expiry (mutually exclusive with --date)
//!     #[arg(long, value_name = "DAYS", conflicts_with = "date")]
//!     pub in_days: Option<u32>,
//!
//!     /// Optional reason appended to the annotation message
//!     #[arg(long, value_name = "TEXT")]
//!     pub reason: Option<String>,
//!
//!     /// Skip the confirmation prompt and write immediately
//!     #[arg(long, default_value_t = false)]
//!     pub yes: bool,
//! }
//! ```
//!
//! Then add a thin wrapper in `cli.rs` / `main.rs` that calls:
//!
//! ```ignore
//! timebomb::snooze::run_snooze(
//!     &args.target,
//!     args.date.as_deref(),
//!     args.in_days,
//!     args.reason.as_deref(),
//!     args.yes,
//!     today,
//! )
//! ```

use crate::error::{Error, Result};
use chrono::{Duration, NaiveDate};
use regex::Regex;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Core logic for `timebomb snooze`.
///
/// All parameters are primitives so this compiles independently of `cli.rs`
/// changes.
///
/// # Parameters
/// - `target`   — `"path/to/file.rs:42"` (file path + colon + 1-based line)
/// - `date_str` — optional `"YYYY-MM-DD"` new expiry date
/// - `in_days`  — optional number of days from `today` until new expiry
/// - `reason`   — optional reason text appended to the annotation
/// - `yes`      — skip confirmation prompt when `true`
/// - `today`    — the current date (injected for testability)
pub fn run_snooze(
    target: &str,
    date_str: Option<&str>,
    in_days: Option<u32>,
    reason: Option<&str>,
    yes: bool,
    today: NaiveDate,
) -> Result<i32> {
    // 1. Parse target --------------------------------------------------------
    let (file_path, line_number) = parse_target(target)?;

    // 2. Resolve the new expiry date ----------------------------------------
    let new_date = resolve_new_date(date_str, in_days, today)?;

    // 3. Read the file -------------------------------------------------------
    let content = std::fs::read_to_string(&file_path).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.clone()),
    })?;

    // 4. Validate line number is in range ------------------------------------
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    if line_number < 1 || line_number > line_count {
        return Err(Error::InvalidArgument(format!(
            "line {} does not exist in file (file has {} lines)",
            line_number, line_count,
        )));
    }

    // 5. Extract the target line (0-indexed) ---------------------------------
    let original_line = lines[line_number - 1];

    // 6. Call snooze_line to replace the date --------------------------------
    let snoozed = snooze_line(original_line, new_date).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "no timebomb date bracket found on line {} of {}",
            line_number,
            file_path.display(),
        ))
    })?;

    // 7. Optionally append reason --------------------------------------------
    let new_line = match reason {
        Some(r) => append_reason(&snoozed, r),
        None => snoozed,
    };

    // 8. Reconstruct the full file -------------------------------------------
    let mut new_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    new_lines[line_number - 1] = new_line.clone();
    let mut new_content = new_lines.join("\n");
    // Preserve trailing newline if the original had one
    if content.ends_with('\n') {
        new_content.push('\n');
    }

    // 9. Print before/after diff ---------------------------------------------
    println!(
        "- {}:{}  {}",
        file_path.display(),
        line_number,
        original_line
    );
    println!("+ {}:{}  {}", file_path.display(), line_number, new_line);

    // 10. Prompt for confirmation (unless --yes) -----------------------------
    if !yes {
        print!("Write change? [y/N]: ");
        io::stdout().flush().map_err(|e| Error::Io {
            source: e,
            path: None,
        })?;

        let stdin = io::stdin();
        let mut line_buf = String::new();
        stdin
            .lock()
            .read_line(&mut line_buf)
            .map_err(|e| Error::Io {
                source: e,
                path: None,
            })?;

        let response = line_buf.trim();
        if response != "y" && response != "Y" {
            return Ok(0);
        }
    }

    // 11. Write the file -----------------------------------------------------
    std::fs::write(&file_path, &new_content).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.clone()),
    })?;

    // 12. Print confirmation -------------------------------------------------
    println!(
        "snoozed {}:{} → {}",
        file_path.display(),
        line_number,
        new_date.format("%Y-%m-%d"),
    );

    // 13. Return success -----------------------------------------------------
    Ok(0)
}

// ---------------------------------------------------------------------------
// Helper: parse_target
// ---------------------------------------------------------------------------

/// Parse `"path/to/file.rs:42"` into `(PathBuf, usize)`.
///
/// Splits on the *last* `:` so that Windows absolute paths (`C:\foo\bar.rs:5`)
/// are handled correctly as long as the user puts the colon-number at the end.
pub fn parse_target(target: &str) -> Result<(PathBuf, usize)> {
    let colon_pos = target.rfind(':').ok_or_else(|| {
        Error::InvalidArgument(format!(
            "target '{}' must be in the form 'file:LINE' (e.g. src/main.rs:42)",
            target
        ))
    })?;

    let file_part = &target[..colon_pos];
    let line_part = &target[colon_pos + 1..];

    if file_part.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "target '{}': file path is empty",
            target
        )));
    }

    let line_number: usize = line_part.parse().map_err(|_| {
        Error::InvalidArgument(format!(
            "target '{}': '{}' is not a valid line number",
            target, line_part
        ))
    })?;

    if line_number == 0 {
        return Err(Error::InvalidArgument(format!(
            "target '{}': line number must be >= 1",
            target
        )));
    }

    Ok((PathBuf::from(file_part), line_number))
}

// ---------------------------------------------------------------------------
// Helper: resolve_new_date
// ---------------------------------------------------------------------------

/// Resolve the new expiry `NaiveDate` from `--date` or `--in-days` arguments.
///
/// `date_str` takes priority if both are somehow provided.
pub fn resolve_new_date(
    date_str: Option<&str>,
    in_days: Option<u32>,
    today: NaiveDate,
) -> Result<NaiveDate> {
    match (date_str, in_days) {
        (Some(s), _) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
            Error::InvalidArgument(format!("'{}' is not a valid date — expected YYYY-MM-DD", s))
        }),
        (None, Some(days)) => {
            let new_date = today + Duration::days(days as i64);
            Ok(new_date)
        }
        (None, None) => Err(Error::InvalidArgument(
            "either --date or --in-days is required".to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Helper: snooze_line
// ---------------------------------------------------------------------------

/// Given a single source line string, find the first occurrence of a date in
/// the pattern `[YYYY-MM-DD]` and replace it with `[{new_date}]`.
///
/// Returns `Some(new_line)` if a replacement was made, `None` if no date
/// bracket was found on the line.
///
/// Only the FIRST bracketed date is replaced (the expiry date), not any
/// subsequent bracket (e.g. an owner bracket like `[alice]`).
pub fn snooze_line(line: &str, new_date: NaiveDate) -> Option<String> {
    // We compile the regex each call — for a CLI tool this is fine.
    let re = Regex::new(r"\[(\d{4}-\d{2}-\d{2})\]").expect("hardcoded regex is valid");

    let mat = re.find(line)?;

    let new_bracket = format!("[{}]", new_date.format("%Y-%m-%d"));
    let new_line = format!(
        "{}{}{}",
        &line[..mat.start()],
        new_bracket,
        &line[mat.end()..]
    );

    Some(new_line)
}

// ---------------------------------------------------------------------------
// Helper: append_reason
// ---------------------------------------------------------------------------

/// Append ` [snoozed: {reason}]` to the end of `line` after trimming trailing
/// whitespace/newline characters.
///
/// Returns the new string without a trailing newline — the caller handles line
/// endings.
pub fn append_reason(line: &str, reason: &str) -> String {
    let trimmed = line.trim_end();
    format!("{} [snoozed: {}]", trimmed, reason)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::fs;
    use tempfile::tempdir;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Fixed "today" used in all tests.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()
    }

    // -- parse_target --------------------------------------------------------

    #[test]
    fn test_parse_target_valid() {
        let (path, line) = parse_target("src/foo.rs:10").unwrap();
        assert_eq!(path, PathBuf::from("src/foo.rs"));
        assert_eq!(line, 10);
    }

    #[test]
    fn test_parse_target_nested_path() {
        let (path, line) = parse_target("a/b/c.rs:1").unwrap();
        assert_eq!(path, PathBuf::from("a/b/c.rs"));
        assert_eq!(line, 1);
    }

    #[test]
    fn test_parse_target_no_colon() {
        let result = parse_target("src/foo.rs");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("file:LINE") || msg.contains("form"));
    }

    #[test]
    fn test_parse_target_line_zero() {
        let result = parse_target("src/foo.rs:0");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(">= 1") || msg.contains("must be"));
    }

    #[test]
    fn test_parse_target_non_numeric() {
        let result = parse_target("src/foo.rs:abc");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("valid line number") || msg.contains("abc"));
    }

    // -- resolve_new_date ----------------------------------------------------

    #[test]
    fn test_resolve_new_date_from_str() {
        let result = resolve_new_date(Some("2026-06-01"), None, today()).unwrap();
        assert_eq!(result, date("2026-06-01"));
    }

    #[test]
    fn test_resolve_new_date_from_in_days() {
        // today = 2025-06-01, +30 days = 2025-07-01
        let result = resolve_new_date(None, Some(30), today()).unwrap();
        assert_eq!(result, date("2025-07-01"));
    }

    #[test]
    fn test_resolve_new_date_neither() {
        let result = resolve_new_date(None, None, today());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("--date") || msg.contains("--in-days") || msg.contains("required"));
    }

    #[test]
    fn test_resolve_new_date_prefers_date_str() {
        // When both are provided, date_str wins
        let result = resolve_new_date(Some("2026-06-01"), Some(30), today()).unwrap();
        assert_eq!(result, date("2026-06-01"));
    }

    #[test]
    fn test_resolve_new_date_invalid_date_str() {
        let result = resolve_new_date(Some("not-a-date"), None, today());
        assert!(result.is_err());
    }

    // -- snooze_line ---------------------------------------------------------

    #[test]
    fn test_snooze_line_basic() {
        let line = "    // TODO[2025-01-15]: remove legacy oauth flow";
        let new_date = date("2026-03-01");
        let result = snooze_line(line, new_date).unwrap();
        assert_eq!(result, "    // TODO[2026-03-01]: remove legacy oauth flow");
    }

    #[test]
    fn test_snooze_line_no_bracket() {
        let line = "    // TODO: plain comment with no date";
        let result = snooze_line(line, date("2026-01-01"));
        assert!(result.is_none());
    }

    #[test]
    fn test_snooze_line_only_replaces_first_bracket() {
        // Owner bracket [alice] must remain untouched
        let line = "    // TODO[2025-01-15][alice]: remove legacy oauth flow";
        let new_date = date("2026-03-01");
        let result = snooze_line(line, new_date).unwrap();
        assert_eq!(
            result,
            "    // TODO[2026-03-01][alice]: remove legacy oauth flow"
        );
        // Ensure [alice] is still there and unchanged
        assert!(result.contains("[alice]"));
        // Ensure the old date is gone
        assert!(!result.contains("2025-01-15"));
    }

    #[test]
    fn test_snooze_line_preserves_rest_of_line() {
        let line = "    // FIXME[2025-03-10]: this is the message text, do not change";
        let new_date = date("2026-12-01");
        let result = snooze_line(line, new_date).unwrap();
        assert!(result.contains("this is the message text, do not change"));
        assert!(result.contains("2026-12-01"));
        assert!(!result.contains("2025-03-10"));
    }

    // -- append_reason -------------------------------------------------------

    #[test]
    fn test_append_reason_basic() {
        let line = "    // TODO[2026-01-01]: msg";
        let result = append_reason(line, "reason");
        assert_eq!(result, "    // TODO[2026-01-01]: msg [snoozed: reason]");
    }

    #[test]
    fn test_append_reason_trims_trailing_whitespace() {
        let line = "    // TODO[2026-01-01]: msg   ";
        let result = append_reason(line, "because");
        // Trailing spaces should be stripped before appending
        assert_eq!(result, "    // TODO[2026-01-01]: msg [snoozed: because]");
    }

    #[test]
    fn test_append_reason_trims_trailing_newline() {
        let line = "    // TODO[2026-01-01]: msg\n";
        let result = append_reason(line, "upstream");
        assert_eq!(result, "    // TODO[2026-01-01]: msg [snoozed: upstream]");
    }

    // -- run_snooze integration tests ----------------------------------------

    #[test]
    fn test_run_snooze_rewrites_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        let content = "fn foo() {}\n// TODO[2025-01-15]: remove me\nfn bar() {}\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:2", file_path.display());
        let result = run_snooze(&target, Some("2026-06-01"), None, None, true, today());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);

        let updated = fs::read_to_string(&file_path).unwrap();
        assert!(updated.contains("2026-06-01"));
        assert!(!updated.contains("2025-01-15"));
        // Other lines untouched
        assert!(updated.contains("fn foo() {}"));
        assert!(updated.contains("fn bar() {}"));
    }

    #[test]
    fn test_run_snooze_no_annotation_on_line() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        let content = "fn foo() {}\nfn bar() {}\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:1", file_path.display());
        let result = run_snooze(&target, Some("2026-06-01"), None, None, true, today());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no timebomb date bracket found") || msg.contains("date bracket"));
    }

    #[test]
    fn test_run_snooze_line_out_of_range() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        let content = "fn foo() {}\nfn bar() {}\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:99", file_path.display());
        let result = run_snooze(&target, Some("2026-06-01"), None, None, true, today());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("99") && (msg.contains("does not exist") || msg.contains("out of range"))
        );
    }

    #[test]
    fn test_run_snooze_with_reason() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        let content =
            "fn alpha() {}\n// TODO[2025-01-15]: remove legacy oauth flow\nfn beta() {}\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:2", file_path.display());
        let result = run_snooze(
            &target,
            Some("2026-03-01"),
            None,
            Some("blocked on upstream release"),
            true,
            today(),
        );
        assert!(result.is_ok());

        let updated = fs::read_to_string(&file_path).unwrap();
        assert!(updated.contains("2026-03-01"));
        assert!(updated.contains("[snoozed: blocked on upstream release]"));
        assert!(!updated.contains("2025-01-15"));
    }

    #[test]
    fn test_run_snooze_uses_in_days() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        let content = "// TODO[2025-01-15]: something\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:1", file_path.display());
        // today = 2025-06-01, +30 days = 2025-07-01
        let result = run_snooze(&target, None, Some(30), None, true, today());
        assert!(result.is_ok());

        let updated = fs::read_to_string(&file_path).unwrap();
        assert!(updated.contains("2025-07-01"));
    }

    #[test]
    fn test_run_snooze_nonexistent_file_returns_io_error() {
        let result = run_snooze(
            "/nonexistent/path/file.rs:1",
            Some("2026-01-01"),
            None,
            None,
            true,
            today(),
        );
        assert!(result.is_err());
        // Should be an Io error
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::Error::Io { .. }));
    }

    #[test]
    fn test_run_snooze_line_1_of_1() {
        // Edge case: single line file, target line 1
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("single.rs");

        let content = "// TODO[2024-12-31]: single line\n";
        fs::write(&file_path, content).unwrap();

        let target = format!("{}:1", file_path.display());
        let result = run_snooze(&target, Some("2026-01-01"), None, None, true, today());
        assert!(result.is_ok());

        let updated = fs::read_to_string(&file_path).unwrap();
        assert!(updated.contains("2026-01-01"));
        assert!(!updated.contains("2024-12-31"));
    }
}
