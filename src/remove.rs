//! Logic for the `timebomb remove` subcommand.
//!
//! Provides two public entry points:
//! - `run_remove`: remove a single annotation line by target or search pattern
//! - `run_remove_all_expired`: remove all expired annotations found by scan

use crate::add::{find_matching_lines, parse_target};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::scanner::scan;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public entry point: remove a single annotation
// ---------------------------------------------------------------------------

/// Remove a single annotation line from a file.
///
/// # Parameters
/// - `target` — `"FILE:LINE"` when search is None; plain file path when search is Some
/// - `search` — optional pattern to locate the annotation
/// - `yes`    — skip confirmation prompt when `true`
pub fn run_remove(target: &str, search: Option<&str>, yes: bool) -> Result<i32> {
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
            1 => (path, matches[0].0),
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

    // 2. Read the file -------------------------------------------------------
    let content = std::fs::read_to_string(&file_path).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.clone()),
    })?;

    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    // 3. Validate line number ------------------------------------------------
    if line_number < 1 || line_number > line_count {
        return Err(Error::InvalidArgument(format!(
            "line {} does not exist in '{}' ({} lines)",
            line_number,
            file_path.display(),
            line_count,
        )));
    }

    let line_content = lines[line_number - 1];

    // 4. Verify it looks like a timebomb annotation --------------------------
    if !is_timebomb_line(line_content) {
        return Err(Error::InvalidArgument(format!(
            "line {} of {} does not appear to be a timebomb annotation",
            line_number,
            file_path.display(),
        )));
    }

    // 5. Display the line to be removed --------------------------------------
    println!(
        "- {}:{}  {}",
        file_path.display(),
        line_number,
        line_content
    );

    // 6. Prompt for confirmation (unless --yes) ------------------------------
    if !yes {
        print!("Remove this line? [y/N]: ");
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

        let response = buf.trim();
        if response != "y" && response != "Y" {
            return Ok(0);
        }
    }

    // 7. Remove the line -----------------------------------------------------
    remove_line(&file_path, line_number)?;

    println!("removed {}:{}", file_path.display(), line_number);

    Ok(0)
}

// ---------------------------------------------------------------------------
// Public entry point: remove all expired annotations
// ---------------------------------------------------------------------------

/// Remove all expired annotation lines from scanned files.
///
/// Groups by file, removes lines from bottom to top so line numbers don't shift.
///
/// # Parameters
/// - `scan_path` — root path to scan
/// - `cfg`       — scanner configuration
/// - `today`     — current date (injected for testability)
/// - `yes`       — skip confirmation prompt when `true`
pub fn run_remove_all_expired(
    scan_path: &Path,
    cfg: &Config,
    today: NaiveDate,
    yes: bool,
) -> Result<i32> {
    // 1. Scan for expired annotations ----------------------------------------
    let result = scan(scan_path, cfg, today)?;
    let expired: Vec<_> = result.expired();

    if expired.is_empty() {
        println!("No expired annotations found.");
        return Ok(0);
    }

    // 2. Print all annotations that will be removed --------------------------
    println!("Annotations to remove:");
    for ann in &expired {
        println!("  - {}:{}  {}", ann.file.display(), ann.line, ann.message);
    }

    // 3. Prompt for confirmation (unless --yes) ------------------------------
    if !yes {
        print!("Remove {} annotation(s)? [y/N]: ", expired.len());
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

        let response = buf.trim();
        if response != "y" && response != "Y" {
            return Ok(0);
        }
    }

    // 4. Group by file, sort lines descending within each file ---------------
    // The annotation file paths are relative to scan_path, so resolve them.
    let mut by_file: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for ann in &expired {
        let abs_path = scan_path.join(&ann.file);
        by_file.entry(abs_path).or_default().push(ann.line);
    }

    // 5. Remove lines from each file (bottom-up to preserve line numbers) ----
    for (file_path, mut line_numbers) in by_file {
        // Sort descending so we remove from bottom up
        line_numbers.sort_unstable_by(|a, b| b.cmp(a));
        line_numbers.dedup();

        for line_number in line_numbers {
            remove_line(&file_path, line_number)?;
        }
        println!("cleaned {}", file_path.display());
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// Helper: remove_line
// ---------------------------------------------------------------------------

/// Remove the line at `line_number` (1-based) from `file_path`.
///
/// Writes the file back without that line.
/// Returns the original line content.
pub fn remove_line(file_path: &Path, line_number: usize) -> Result<String> {
    let content = std::fs::read_to_string(file_path).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.to_path_buf()),
    })?;

    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    if line_number < 1 || line_number > line_count {
        return Err(Error::InvalidArgument(format!(
            "line {} is out of range for '{}' ({} lines)",
            line_number,
            file_path.display(),
            line_count,
        )));
    }

    let original = lines[line_number - 1].to_string();

    // Build new content without that line
    let mut new_lines: Vec<&str> = lines[..line_number - 1].to_vec();
    new_lines.extend_from_slice(&lines[line_number..]);

    let mut new_content = new_lines.join("\n");
    // Preserve trailing newline if the original had one
    if content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    } else if new_content.is_empty() && content.ends_with('\n') {
        // File is now empty but had a trailing newline — keep it empty
    }

    std::fs::write(file_path, new_content).map_err(|e| Error::Io {
        source: e,
        path: Some(file_path.to_path_buf()),
    })?;

    Ok(original)
}

// ---------------------------------------------------------------------------
// Helper: is_timebomb_line
// ---------------------------------------------------------------------------

/// Returns true if the line contains a timebomb date bracket `[YYYY-MM-DD]`.
fn is_timebomb_line(line: &str) -> bool {
    // Simple check: look for [YYYY-MM-DD] pattern
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            // Try to read YYYY-MM-DD]
            let rest: String = chars.clone().take(11).collect();
            if rest.len() == 11 {
                let date_part = &rest[..10];
                let close = rest.chars().nth(10);
                if close == Some(']') && looks_like_date(date_part) {
                    return true;
                }
            }
        }
    }
    false
}

/// Quick check if a string looks like YYYY-MM-DD.
fn looks_like_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use tempfile::tempdir;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 22).unwrap()
    }

    // -- remove_line ---------------------------------------------------------

    #[test]
    fn test_remove_line_basic() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "line one\nline two\nline three\n").unwrap();

        let original = remove_line(&file, 2).unwrap();
        assert_eq!(original, "line two");

        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "line one");
        assert_eq!(lines[1], "line three");
    }

    #[test]
    fn test_remove_line_first_line() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "first\nsecond\nthird\n").unwrap();

        let original = remove_line(&file, 1).unwrap();
        assert_eq!(original, "first");

        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "second");
        assert_eq!(lines[1], "third");
    }

    #[test]
    fn test_remove_line_last_line() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "first\nsecond\nlast\n").unwrap();

        let original = remove_line(&file, 3).unwrap();
        assert_eq!(original, "last");

        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1], "second");
    }

    #[test]
    fn test_remove_line_out_of_range() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "only line\n").unwrap();

        let result = remove_line(&file, 99);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("out of range") || msg.contains("99"));
    }

    // -- run_remove ----------------------------------------------------------

    #[test]
    fn test_run_remove_removes_annotation() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(
            &file,
            "fn alpha() {}\n// TODO[2020-01-01]: expired remove\nfn beta() {}\n",
        )
        .unwrap();

        let target = format!("{}:2", file.display());
        let result = run_remove(&target, None, true);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "fn alpha() {}");
        assert_eq!(lines[1], "fn beta() {}");
    }

    #[test]
    fn test_run_remove_non_annotation_line() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn alpha() {}\nfn beta() {}\n").unwrap();

        let target = format!("{}:1", file.display());
        let result = run_remove(&target, None, true);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("does not appear to be a timebomb annotation"));
    }

    #[test]
    fn test_run_remove_by_search_single_match() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(
            &file,
            "fn alpha() {}\n// TODO[2020-01-01]: legacy_auth remove\nfn beta() {}\n",
        )
        .unwrap();

        let result = run_remove(file.to_str().unwrap(), Some("legacy_auth"), true);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&file).unwrap();
        assert!(!content.contains("legacy_auth"));
        assert!(content.contains("fn alpha()"));
        assert!(content.contains("fn beta()"));
    }

    #[test]
    fn test_run_remove_by_search_no_match() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn alpha() {}\nfn beta() {}\n").unwrap();

        let result = run_remove(file.to_str().unwrap(), Some("zzz_no_match"), true);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no lines matching"));
    }

    #[test]
    fn test_run_remove_by_search_multiple_matches() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(
            &file,
            "// TODO[2020-01-01]: foo one\n// TODO[2020-02-01]: foo two\n",
        )
        .unwrap();

        let result = run_remove(file.to_str().unwrap(), Some("foo"), true);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("matched") || msg.contains("lines"));
    }

    // -- run_remove_all_expired ----------------------------------------------

    #[test]
    fn test_run_remove_all_expired_no_expired() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("ok.rs");
        std::fs::write(&file, "// TODO[2099-01-01]: far future\n").unwrap();

        let cfg = crate::config::Config::default();
        let result = run_remove_all_expired(dir.path(), &cfg, today(), true);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_run_remove_all_expired_removes_from_multiple_files() {
        let dir = tempdir().unwrap();

        let file_a = dir.path().join("a.rs");
        std::fs::write(
            &file_a,
            "fn foo() {}\n// TODO[2020-01-01]: expired a\nfn bar() {}\n",
        )
        .unwrap();

        let file_b = dir.path().join("b.rs");
        std::fs::write(&file_b, "// TODO[2019-06-01]: expired b\nfn baz() {}\n").unwrap();

        let cfg = crate::config::Config::default();
        let result = run_remove_all_expired(dir.path(), &cfg, today(), true);
        assert!(result.is_ok());

        let content_a = std::fs::read_to_string(&file_a).unwrap();
        assert!(!content_a.contains("expired a"));
        assert!(content_a.contains("fn foo()"));
        assert!(content_a.contains("fn bar()"));

        let content_b = std::fs::read_to_string(&file_b).unwrap();
        assert!(!content_b.contains("expired b"));
        assert!(content_b.contains("fn baz()"));
    }

    #[test]
    fn test_run_remove_all_expired_multiline_file_line_numbers_correct() {
        // 3 expired annotations in one file — all should be removed cleanly
        let dir = tempdir().unwrap();
        let file = dir.path().join("multi.rs");
        std::fs::write(
            &file,
            "fn a() {}\n\
             // TODO[2020-01-01]: first expired\n\
             fn b() {}\n\
             // FIXME[2019-06-01]: second expired\n\
             fn c() {}\n\
             // HACK[2018-03-01]: third expired\n\
             fn d() {}\n",
        )
        .unwrap();

        let cfg = crate::config::Config::default();
        let result = run_remove_all_expired(dir.path(), &cfg, today(), true);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&file).unwrap();
        assert!(!content.contains("first expired"));
        assert!(!content.contains("second expired"));
        assert!(!content.contains("third expired"));
        assert!(content.contains("fn a()"));
        assert!(content.contains("fn b()"));
        assert!(content.contains("fn c()"));
        assert!(content.contains("fn d()"));
    }
}
