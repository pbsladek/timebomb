//! Logic for the `timebomb plant` subcommand.
//!
//! This module implements the core logic for inserting a timebomb fuse into a
//! source file at a specific line. It is intentionally defined with primitive
//! parameters so that it compiles independently of any `PlantArgs` struct
//! changes in `cli.rs`.

use crate::error::{Error, Result};
use chrono::NaiveDate;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Core logic for `timebomb plant`.
///
/// All parameters are primitives so this compiles independently of `cli.rs`
/// changes.
///
/// # Parameters
/// - `target`   — `"path/to/file.rs:42"` when search is None; plain file path when search is Some
/// - `tag`      — tag keyword, e.g. `"TODO"`
/// - `owner`    — optional owner name
/// - `date_str` — optional `"YYYY-MM-DD"` expiry date
/// - `in_days`  — optional number of days from `today` until expiry
/// - `yes`      — skip confirmation prompt when `true`
/// - `message`  — annotation message text
/// - `today`    — the current date (injected for testability)
/// - `search`   — optional pattern; when Some, `target` is a plain file path
#[allow(clippy::too_many_arguments)]
pub fn run_add(
    target: &str,
    tag: &str,
    owner: Option<&str>,
    date_str: Option<&str>,
    in_days: Option<u32>,
    yes: bool,
    message: &str,
    today: NaiveDate,
    search: Option<&str>,
) -> Result<i32> {
    // 1. Resolve file path and line number -----------------------------------
    let (file_path, line_number) = if let Some(pattern) = search {
        let path = PathBuf::from(target);
        let matches = find_matching_lines(&path, pattern)?;
        match matches.len() {
            0 => {
                return Err(Error::InvalidArgument(format!(
                    "no lines matching '{}' found in {}",
                    pattern, target
                )));
            }
            1 => {
                println!("matched line {}: {}", matches[0].0, matches[0].1.trim_end());
                (path, matches[0].0)
            }
            n => {
                let mut detail =
                    format!("pattern '{}' matched {} lines in {}:", pattern, n, target);
                for (ln, content) in &matches {
                    detail.push_str(&format!("\n  line {}: {}", ln, content.trim_end()));
                }
                detail.push_str("\nuse FILE:LINE to be specific");
                return Err(Error::InvalidArgument(detail));
            }
        }
    } else {
        parse_target(target)?
    };

    // 2. Resolve the expiry date ---------------------------------------------
    let expiry = resolve_date(date_str, in_days, today, yes)?;

    // Warn if the date is already in the past — the fuse will immediately detonate.
    if expiry < today {
        eprintln!(
            "warning: expiry date {} is already in the past — this fuse will detonate immediately",
            expiry.format("%Y-%m-%d")
        );
    }

    // 3. Detect comment style ------------------------------------------------
    let prefix = detect_comment_style(&file_path);

    // 4. Build the annotation string -----------------------------------------
    let annotation = build_annotation(prefix, tag, expiry, owner, message);

    // 5. Read the file -------------------------------------------------------
    let content = std::fs::read_to_string(&file_path).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.clone()),
    })?;
    let had_trailing_newline = content.ends_with('\n');

    // 6. Validate line number ------------------------------------------------
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    // line_number is 1-based; allow inserting at line_count + 1 (append)
    if line_number < 1 || line_number > line_count + 1 {
        return Err(Error::InvalidArgument(format!(
            "line number {} is out of range for '{}' ({} lines); \
             must be between 1 and {}",
            line_number,
            file_path.display(),
            line_count,
            line_count + 1,
        )));
    }

    // 7. Build the new file content ------------------------------------------
    let mut new_content = insert_line(&lines, line_number, &annotation);
    // insert_line always appends a trailing newline; strip it if the original
    // file did not have one so we don't alter the file's newline convention.
    if !had_trailing_newline {
        new_content.pop();
    }

    // 8. Print a diff --------------------------------------------------------
    println!("+ {}:{}  {}", file_path.display(), line_number, annotation);

    // 9. Prompt for confirmation (unless --yes) ------------------------------
    if !yes {
        print!("Write change? [y/N]: ");
        // Flush so the prompt appears before we block on stdin.
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
            // User cancelled — not an error.
            return Ok(0);
        }
    }

    // 10. Write the file atomically ------------------------------------------
    // Write to a sibling temp file then rename so a mid-write crash never
    // leaves a partially-written source file.
    let tmp_path = file_path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, &new_content).map_err(|e| Error::Io {
        source: e,
        path: Some(tmp_path.clone()),
    })?;
    std::fs::rename(&tmp_path, &file_path).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.clone()),
    })?;

    // 11. Print confirmation -------------------------------------------------
    println!("wrote {}:{}", file_path.display(), line_number);

    // 12. Return success -----------------------------------------------------
    Ok(0)
}

// ---------------------------------------------------------------------------
// Helper: parse_target
// ---------------------------------------------------------------------------

/// Parse `"path/to/file.rs:42"` into `(PathBuf, usize)`.
///
/// Accepts optional column and trailing editor context after the line number:
/// - `src/foo.rs:42`
/// - `src/foo.rs:42:7`
/// - `src/foo.rs:42:7: some editor context`
///
/// Splits on the *last* `:` so that Windows absolute paths (`C:\foo\bar.rs:5`)
/// are handled correctly as long as the user puts the colon-number at the end.
pub fn parse_target(target: &str) -> Result<(PathBuf, usize)> {
    // Find the last colon
    let last_colon = target.rfind(':').ok_or_else(|| {
        Error::InvalidArgument(format!(
            "target '{}' must be in the form 'file:LINE' (e.g. src/main.rs:42)",
            target
        ))
    })?;

    let last_segment = &target[last_colon + 1..];

    // Check if the last segment is purely digits (possibly with trailing spaces)
    // If it is, it might be a column number — look back further.
    if last_segment.trim().chars().all(|c| c.is_ascii_digit()) && !last_segment.trim().is_empty() {
        // Could be file:line or file:line:col
        // Check if there is another colon before this position
        let before_last = &target[..last_colon];
        if let Some(prev_colon) = before_last.rfind(':') {
            let prev_segment = &before_last[prev_colon + 1..];
            // If prev_segment is also purely digits, treat it as the line number
            // and last_segment as the column
            if prev_segment.trim().chars().all(|c| c.is_ascii_digit())
                && !prev_segment.trim().is_empty()
            {
                let file_part = &before_last[..prev_colon];
                let line_part = prev_segment.trim();

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

                return Ok((PathBuf::from(file_part), line_number));
            }
        }
        // Fall through: the last segment is a line number (no col present)
    } else if !last_segment.trim().is_empty() {
        // Last segment starts with digits but has trailing non-digit content
        // e.g. "42:7: some editor context" — walk back to find the line number
        // Actually handle: "file:42:7: some text" where last colon is before "some text"
        // but last_segment is not purely digits.
        // Try: strip trailing text after space/colon, find line number in second-to-last numeric segment
        let before_last = &target[..last_colon];
        if let Some(prev_colon) = before_last.rfind(':') {
            let prev_segment = &before_last[prev_colon + 1..];
            if prev_segment.trim().chars().all(|c| c.is_ascii_digit())
                && !prev_segment.trim().is_empty()
            {
                // prev_segment is purely digits — check if the segment before it is also digits (col)
                let before_prev = &before_last[..prev_colon];
                if let Some(pp_colon) = before_prev.rfind(':') {
                    let pp_segment = &before_prev[pp_colon + 1..];
                    if pp_segment.trim().chars().all(|c| c.is_ascii_digit())
                        && !pp_segment.trim().is_empty()
                    {
                        // file:line:col: trailing text
                        let file_part = &before_prev[..pp_colon];
                        let line_part = pp_segment.trim();

                        if !file_part.is_empty() {
                            let line_number: usize = line_part.parse().map_err(|_| {
                                Error::InvalidArgument(format!(
                                    "target '{}': '{}' is not a valid line number",
                                    target, line_part
                                ))
                            })?;
                            if line_number > 0 {
                                return Ok((PathBuf::from(file_part), line_number));
                            }
                        }
                    }
                }
                // prev_segment is line, last_segment is trailing text (not digits)
                let file_part = &before_prev;
                let line_part = prev_segment.trim();
                if !file_part.is_empty() {
                    let line_number: usize = line_part.parse().map_err(|_| {
                        Error::InvalidArgument(format!(
                            "target '{}': '{}' is not a valid line number",
                            target, line_part
                        ))
                    })?;
                    if line_number > 0 {
                        return Ok((PathBuf::from(*file_part), line_number));
                    }
                }
            }
        }
    }

    // Standard file:line parsing
    let file_part = &target[..last_colon];
    let line_part = last_segment.trim();

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
// Helper: find_matching_lines
// ---------------------------------------------------------------------------

/// Scan a file line-by-line and return all (1-based line number, line content)
/// pairs where the line contains `pattern` as a substring (case-sensitive).
pub fn find_matching_lines(file: &Path, pattern: &str) -> Result<Vec<(usize, String)>> {
    let content = std::fs::read_to_string(file).map_err(|e| Error::Io {
        source: e,
        path: Some(file.to_path_buf()),
    })?;

    let matches = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pattern))
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect();

    Ok(matches)
}

// ---------------------------------------------------------------------------
// Helper: resolve_date
// ---------------------------------------------------------------------------

/// Resolve the expiry `NaiveDate` from the `--date` or `--in-days` arguments.
///
/// When both are None:
/// - If `yes` is true: default to 90 days silently (prints a notice)
/// - If `yes` is false: prompt the user for a number of days (default 90)
pub fn resolve_date(
    date_str: Option<&str>,
    in_days: Option<u32>,
    today: NaiveDate,
    yes: bool,
) -> Result<NaiveDate> {
    match (date_str, in_days) {
        (Some(s), _) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
            Error::InvalidArgument(format!("'{}' is not a valid date — expected YYYY-MM-DD", s))
        }),
        (None, Some(days)) => today
            .checked_add_signed(chrono::Duration::days(days as i64))
            .ok_or_else(|| {
                Error::InvalidArgument(format!("--in-days {} overflows the calendar", days))
            }),
        (None, None) => {
            let days: u32 = if yes {
                let default_date = today
                    .checked_add_signed(chrono::Duration::days(90))
                    .ok_or_else(|| {
                        Error::InvalidArgument("90-day default overflows the calendar".to_string())
                    })?;
                println!(
                    "No expiry specified; defaulting to 90 days from today ({})",
                    default_date.format("%Y-%m-%d")
                );
                90
            } else {
                print!("Expire in how many days? [90]: ");
                io::stdout().flush().map_err(|e| Error::Io {
                    source: e,
                    path: None,
                })?;
                let stdin = io::stdin();
                let mut buf = String::new();
                stdin.lock().read_line(&mut buf).map_err(|e| Error::Io {
                    source: e,
                    path: None,
                })?;
                let trimmed = buf.trim();
                if trimmed.is_empty() {
                    90
                } else {
                    trimmed.parse::<u32>().map_err(|_| {
                        Error::InvalidArgument(format!(
                            "'{}' is not a valid number of days",
                            trimmed
                        ))
                    })?
                }
            };
            today
                .checked_add_signed(chrono::Duration::days(days as i64))
                .ok_or_else(|| {
                    Error::InvalidArgument(format!("--in-days {} overflows the calendar", days))
                })
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: detect_comment_style
// ---------------------------------------------------------------------------

/// Return the comment prefix appropriate for the given file extension.
///
/// | Prefix | Extensions                                                            |
/// |--------|-----------------------------------------------------------------------|
/// | `//`   | rs, go, ts, js, jsx, tsx, java, swift, c, cpp, cc, cs, kt            |
/// | `#`    | py, rb, sh, bash, zsh, yaml, yml, tf, toml, r                        |
/// | `--`   | sql, lua, hs                                                          |
/// | `//`   | anything else (default)                                               |
pub fn detect_comment_style(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        // C-style line comments
        "rs" | "go" | "ts" | "js" | "jsx" | "tsx" | "java" | "swift" | "c" | "cpp" | "cc"
        | "cs" | "kt" => "//",

        // Hash-style comments
        "py" | "rb" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "tf" | "toml" | "r" => "#",

        // Double-dash comments
        "sql" | "lua" | "hs" => "--",

        // Default to C-style
        _ => "//",
    }
}

// ---------------------------------------------------------------------------
// Helper: build_annotation
// ---------------------------------------------------------------------------

/// Build the full annotation string.
///
/// Without owner: `{prefix} {TAG}[{YYYY-MM-DD}]: {message}`
/// With owner:    `{prefix} {TAG}[{YYYY-MM-DD}][{owner}]: {message}`
pub fn build_annotation(
    prefix: &str,
    tag: &str,
    expiry: NaiveDate,
    owner: Option<&str>,
    message: &str,
) -> String {
    let tag_upper = tag.to_uppercase();
    let date_str = expiry.format("%Y-%m-%d");
    match owner {
        None => format!("{} {}[{}]: {}", prefix, tag_upper, date_str, message),
        Some(o) => format!("{} {}[{}][{}]: {}", prefix, tag_upper, date_str, o, message),
    }
}

// ---------------------------------------------------------------------------
// Helper: insert_line
// ---------------------------------------------------------------------------

/// Insert `new_line` *before* the 1-based `line_number` in `lines`, returning
/// the complete new file content as a `String`.
///
/// Inserting at `line_number == lines.len() + 1` appends after the last line.
///
/// The returned string always ends with a newline.
pub fn insert_line(lines: &[&str], line_number: usize, new_line: &str) -> String {
    // Convert to owned strings so we can splice freely.
    let mut owned: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

    // line_number is 1-based; insert at index (line_number - 1).
    let insert_at = line_number - 1;
    owned.insert(insert_at, new_line.to_string());

    // Join with newlines and ensure trailing newline.
    let mut result = owned.join("\n");
    result.push('\n');
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 22).unwrap()
    }

    // -- detect_comment_style ------------------------------------------------

    #[test]
    fn test_detect_comment_style_rs() {
        assert_eq!(
            detect_comment_style(std::path::Path::new("src/main.rs")),
            "//"
        );
    }

    #[test]
    fn test_detect_comment_style_py() {
        assert_eq!(detect_comment_style(std::path::Path::new("script.py")), "#");
    }

    #[test]
    fn test_detect_comment_style_sql() {
        assert_eq!(
            detect_comment_style(std::path::Path::new("schema.sql")),
            "--"
        );
    }

    #[test]
    fn test_detect_comment_style_unknown() {
        assert_eq!(detect_comment_style(std::path::Path::new("file.xyz")), "//");
    }

    #[test]
    fn test_detect_comment_style_no_extension() {
        assert_eq!(detect_comment_style(std::path::Path::new("Makefile")), "//");
    }

    #[test]
    fn test_detect_comment_style_go() {
        assert_eq!(detect_comment_style(std::path::Path::new("main.go")), "//");
    }

    #[test]
    fn test_detect_comment_style_yaml() {
        assert_eq!(
            detect_comment_style(std::path::Path::new("config.yaml")),
            "#"
        );
    }

    #[test]
    fn test_detect_comment_style_lua() {
        assert_eq!(detect_comment_style(std::path::Path::new("init.lua")), "--");
    }

    #[test]
    fn test_detect_comment_style_toml() {
        assert_eq!(
            detect_comment_style(std::path::Path::new("Cargo.toml")),
            "#"
        );
    }

    // -- build_annotation ----------------------------------------------------

    #[test]
    fn test_build_annotation_no_owner() {
        let expiry = date("2026-09-01");
        let result = build_annotation("//", "todo", expiry, None, "remove legacy oauth flow");
        assert_eq!(result, "// TODO[2026-09-01]: remove legacy oauth flow");
    }

    #[test]
    fn test_build_annotation_with_owner() {
        let expiry = date("2026-09-01");
        let result = build_annotation(
            "//",
            "TODO",
            expiry,
            Some("alice"),
            "remove legacy oauth flow",
        );
        assert_eq!(
            result,
            "// TODO[2026-09-01][alice]: remove legacy oauth flow"
        );
    }

    #[test]
    fn test_build_annotation_tag_uppercased() {
        let expiry = date("2027-01-15");
        let result = build_annotation("#", "fixme", expiry, None, "cleanup");
        assert_eq!(result, "# FIXME[2027-01-15]: cleanup");
    }

    #[test]
    fn test_build_annotation_sql_prefix() {
        let expiry = date("2025-12-31");
        let result = build_annotation("--", "HACK", expiry, Some("bob"), "temp workaround");
        assert_eq!(result, "-- HACK[2025-12-31][bob]: temp workaround");
    }

    // -- parse_target --------------------------------------------------------

    #[test]
    fn test_parse_target_valid() {
        let (path, line) = parse_target("src/foo.rs:42").unwrap();
        assert_eq!(path, PathBuf::from("src/foo.rs"));
        assert_eq!(line, 42);
    }

    #[test]
    fn test_parse_target_valid_nested() {
        let (path, line) = parse_target("a/b/c/main.go:1").unwrap();
        assert_eq!(path, PathBuf::from("a/b/c/main.go"));
        assert_eq!(line, 1);
    }

    #[test]
    fn test_parse_target_invalid_no_colon() {
        let result = parse_target("src/foo.rs");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("FILE:LINE") || msg.contains("file:LINE") || msg.contains("form"));
    }

    #[test]
    fn test_parse_target_invalid_line_zero() {
        let result = parse_target("src/foo.rs:0");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("1") || msg.contains("zero") || msg.contains(">="));
    }

    #[test]
    fn test_parse_target_invalid_non_numeric_line() {
        let result = parse_target("src/foo.rs:abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_target_empty_file() {
        let result = parse_target(":42");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_target_accepts_col() {
        // file:line:col — should return line=42
        let (path, line) = parse_target("src/foo.rs:42:7").unwrap();
        assert_eq!(path, PathBuf::from("src/foo.rs"));
        assert_eq!(line, 42);
    }

    #[test]
    fn test_parse_target_accepts_col_and_message() {
        // file:line:col: trailing message — should return line=42
        let (path, line) = parse_target("src/foo.rs:42:7: some editor context").unwrap();
        assert_eq!(path, PathBuf::from("src/foo.rs"));
        assert_eq!(line, 42);
    }

    // -- resolve_date --------------------------------------------------------

    #[test]
    fn test_resolve_date_from_date_str() {
        let t = date("2025-06-01");
        let result = resolve_date(Some("2026-09-01"), None, t, true).unwrap();
        assert_eq!(result, date("2026-09-01"));
    }

    #[test]
    fn test_resolve_date_from_in_days() {
        let t = date("2025-06-01");
        let result = resolve_date(None, Some(90), t, true).unwrap();
        assert_eq!(result, date("2025-08-30"));
    }

    #[test]
    fn test_resolve_date_in_days_zero() {
        let t = date("2025-06-01");
        let result = resolve_date(None, Some(0), t, true).unwrap();
        assert_eq!(result, t);
    }

    #[test]
    fn test_resolve_date_neither_yes_defaults_90() {
        // When neither --date nor --in-days and yes=true, defaults to 90 days
        let t = today();
        let result = resolve_date(None, None, t, true).unwrap();
        let expected = t.checked_add_signed(chrono::Duration::days(90)).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_resolve_date_prefers_date_str_over_in_days() {
        let t = date("2025-06-01");
        let result = resolve_date(Some("2099-01-01"), Some(5), t, true).unwrap();
        assert_eq!(result, date("2099-01-01"));
    }

    #[test]
    fn test_resolve_date_invalid_format() {
        let t = date("2025-06-01");
        let result = resolve_date(Some("01-09-2026"), None, t, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_add_default_days_yes() {
        // Neither --date nor --in-days, yes=true → defaults to 90 days
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let target = format!("{}:1", file.display());

        let t = today();
        let result = run_add(&target, "TODO", None, None, None, true, "msg", t, None);
        assert!(result.is_ok());

        let written = std::fs::read_to_string(&file).unwrap();
        let expected_date = t
            .checked_add_signed(chrono::Duration::days(90))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        assert!(written.contains(&expected_date));
    }

    // -- insert_line ---------------------------------------------------------

    #[test]
    fn test_insert_line_middle() {
        // 3-line file, insert at line 2 → new line becomes line 2, old line 2 → line 3
        let lines = vec!["line one", "line two", "line three"];
        let result = insert_line(&lines, 2, "// TODO[2026-01-01]: new annotation");
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 4);
        assert_eq!(result_lines[0], "line one");
        assert_eq!(result_lines[1], "// TODO[2026-01-01]: new annotation");
        assert_eq!(result_lines[2], "line two");
        assert_eq!(result_lines[3], "line three");
    }

    #[test]
    fn test_insert_line_first() {
        let lines = vec!["first", "second", "third"];
        let result = insert_line(&lines, 1, "// annotation");
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 4);
        assert_eq!(result_lines[0], "// annotation");
        assert_eq!(result_lines[1], "first");
        assert_eq!(result_lines[2], "second");
        assert_eq!(result_lines[3], "third");
    }

    #[test]
    fn test_insert_line_after_last() {
        // Inserting at line N+1 appends
        let lines = vec!["alpha", "beta", "gamma"];
        let result = insert_line(&lines, 4, "// appended");
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 4);
        assert_eq!(result_lines[3], "// appended");
    }

    #[test]
    fn test_insert_line_single_line_file() {
        let lines = vec!["only line"];
        let result = insert_line(&lines, 1, "// before");
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 2);
        assert_eq!(result_lines[0], "// before");
        assert_eq!(result_lines[1], "only line");
    }

    #[test]
    fn test_insert_line_trailing_newline() {
        let lines = vec!["a", "b"];
        let result = insert_line(&lines, 1, "x");
        assert!(result.ends_with('\n'), "result should end with a newline");
    }

    // -- find_matching_lines -------------------------------------------------

    #[test]
    fn test_find_matching_lines_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "line one\ncontains_pattern here\nline three\n").unwrap();
        let matches = find_matching_lines(&file, "contains_pattern").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, 2);
        assert!(matches[0].1.contains("contains_pattern"));
    }

    #[test]
    fn test_find_matching_lines_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "foo bar\nfoo baz\nno match\n").unwrap();
        let matches = find_matching_lines(&file, "foo").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].0, 1);
        assert_eq!(matches[1].0, 2);
    }

    #[test]
    fn test_find_matching_lines_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "line one\nline two\n").unwrap();
        let matches = find_matching_lines(&file, "zzz_no_match").unwrap();
        assert_eq!(matches.len(), 0);
    }

    // -- run_add (integration tests using tempfile) --------------------------

    #[test]
    fn test_run_add_invalid_target_no_colon() {
        let t = date("2025-06-01");
        let result = run_add(
            "src/nocoton",
            "TODO",
            None,
            Some("2026-01-01"),
            None,
            true,
            "msg",
            t,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_run_add_missing_date_and_in_days_yes_defaults() {
        // With yes=true and no date/in_days, should default to 90 days (not error)
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let target = format!("{}:1", file.display());

        let t = date("2025-06-01");
        let result = run_add(&target, "TODO", None, None, None, true, "msg", t, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_add_line_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let target = format!("{}:999", file.display());

        let t = date("2025-06-01");
        let result = run_add(
            &target,
            "TODO",
            None,
            Some("2026-01-01"),
            None,
            true,
            "msg",
            t,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_run_add_inserts_annotation_with_yes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn foo() {}\nfn bar() {}\n").unwrap();
        let target = format!("{}:1", file.display());

        let t = date("2025-06-01");
        let result = run_add(
            &target,
            "TODO",
            None,
            Some("2026-09-01"),
            None,
            true, // --yes: skip prompt
            "remove foo after migration",
            t,
            None,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);

        let written = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "// TODO[2026-09-01]: remove foo after migration");
        assert_eq!(lines[1], "fn foo() {}");
        assert_eq!(lines[2], "fn bar() {}");
    }

    #[test]
    fn test_run_add_inserts_annotation_with_owner() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "def foo():\n    pass\n").unwrap();
        let target = format!("{}:2", file.display());

        let t = date("2025-06-01");
        let result = run_add(
            &target,
            "FIXME",
            Some("alice"),
            None,
            Some(30),
            true,
            "clean this up",
            t,
            None,
        );
        assert!(result.is_ok());

        let written = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "def foo():");
        // date should be today + 30 days = 2025-07-01
        assert_eq!(lines[1], "# FIXME[2025-07-01][alice]: clean this up");
        assert_eq!(lines[2], "    pass");
    }

    #[test]
    fn test_run_add_append_after_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "line1\nline2\n").unwrap();
        // line count is 2; line 3 = append
        let target = format!("{}:3", file.display());

        let t = date("2025-06-01");
        let result = run_add(
            &target,
            "TODO",
            None,
            Some("2027-01-01"),
            None,
            true,
            "appended",
            t,
            None,
        );
        assert!(result.is_ok());

        let written = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2], "// TODO[2027-01-01]: appended");
    }

    #[test]
    fn test_run_add_nonexistent_file_returns_io_error() {
        let t = date("2025-06-01");
        let result = run_add(
            "/nonexistent/path/file.rs:1",
            "TODO",
            None,
            Some("2026-01-01"),
            None,
            true,
            "msg",
            t,
            None,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Io { .. } => {}
            other => panic!("expected Io error, got: {:?}", other),
        }
    }

    #[test]
    fn test_run_add_with_search_single_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn alpha() {}\nfn legacy_auth() {}\nfn gamma() {}\n").unwrap();

        let t = today();
        let result = run_add(
            file.to_str().unwrap(),
            "TODO",
            None,
            Some("2027-01-01"),
            None,
            true,
            "remove legacy auth",
            t,
            Some("legacy_auth"),
        );
        assert!(result.is_ok());

        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.contains("TODO[2027-01-01]: remove legacy auth"));
    }

    #[test]
    fn test_run_add_with_search_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn alpha() {}\nfn beta() {}\n").unwrap();

        let t = today();
        let result = run_add(
            file.to_str().unwrap(),
            "TODO",
            None,
            Some("2027-01-01"),
            None,
            true,
            "msg",
            t,
            Some("zzz_no_match"),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no lines matching"));
    }

    #[test]
    fn test_run_add_with_search_multiple_matches() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn foo_a() {}\nfn foo_b() {}\nfn bar() {}\n").unwrap();

        let t = today();
        let result = run_add(
            file.to_str().unwrap(),
            "TODO",
            None,
            Some("2027-01-01"),
            None,
            true,
            "msg",
            t,
            Some("foo"),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("matched") || msg.contains("2 lines"));
    }
}
