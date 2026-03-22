use crate::annotation::Annotation;
use crate::config::Config;
use crate::error::{Error, Result};
use chrono::NaiveDate;
use rayon::prelude::*;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

/// Result of a full scan run.
#[derive(Debug)]
pub struct ScanResult {
    pub annotations: Vec<Annotation>,
    pub scanned_files: usize,
    pub skipped_files: usize,
}

impl ScanResult {
    pub fn expired(&self) -> Vec<&Annotation> {
        self.annotations.iter().filter(|a| a.is_expired()).collect()
    }

    pub fn expiring_soon(&self) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.is_expiring_soon())
            .collect()
    }

    pub fn ok(&self) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.status == crate::annotation::Status::Ok)
            .collect()
    }

    pub fn has_expired(&self) -> bool {
        self.annotations.iter().any(|a| a.is_expired())
    }

    pub fn has_expiring_soon(&self) -> bool {
        self.annotations.iter().any(|a| a.is_expiring_soon())
    }

    pub fn total(&self) -> usize {
        self.annotations.len()
    }
}

/// Core scanner: walks `root`, respects config, and returns all found annotations.
///
/// `today` is injected rather than derived internally so that tests can use a
/// fixed date without depending on the current wall-clock time.
pub fn scan(root: &Path, config: &Config, today: NaiveDate) -> Result<ScanResult> {
    let globset = config.build_exclude_globset()?;
    let regex = build_regex(config)?;

    // ----------------------------------------------------------------
    // Phase 1 (serial): Walk the directory tree and collect the set of
    // candidate files that pass the cheap exclude/extension/binary
    // filters.  WalkDir is inherently serial (lazy recursion), so we
    // keep this phase single-threaded and use it purely to decide *what*
    // to process.
    // ----------------------------------------------------------------
    struct Candidate {
        abs_path: PathBuf,
        rel_path: PathBuf,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut skipped_files: usize = 0;

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| match e {
            Ok(entry) => Some(entry),
            Err(err) => {
                eprintln!("warning: skipping inaccessible path: {}", err);
                None
            }
        })
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path().to_path_buf();

        // Compute a path relative to root for glob matching and display.
        let rel_path = abs_path
            .strip_prefix(root)
            .unwrap_or(&abs_path)
            .to_path_buf();

        // Skip excluded paths.
        if config.is_excluded(&rel_path, &globset) {
            skipped_files += 1;
            continue;
        }

        // Skip files whose extension is not in the allowed list.
        if !config.extension_allowed(&rel_path) {
            continue;
        }

        // If --since was given, skip files not in the git-diff set.
        if let Some(ref diff_files) = config.diff_files {
            if !diff_files.contains(&rel_path) {
                continue;
            }
        }

        candidates.push(Candidate { abs_path, rel_path });
    }

    // ----------------------------------------------------------------
    // Phase 2 (parallel): Scan each candidate file on a rayon thread-
    // pool worker.  Each worker reads the file once as raw bytes,
    // performs binary detection inline (no second open), then decodes
    // and scans.  Binary skips are counted via an atomic so Phase 1
    // stays free of file I/O.
    // ----------------------------------------------------------------
    let binary_count = AtomicUsize::new(0);
    let results: Result<Vec<Vec<Annotation>>> = candidates
        .par_iter()
        .map(|c| {
            let bytes = std::fs::read(&c.abs_path).map_err(|e| Error::Io {
                source: e,
                path: Some(c.abs_path.clone()),
            })?;
            // Binary detection: a null byte means this is not a text file.
            if bytes.contains(&0u8) {
                binary_count.fetch_add(1, Ordering::Relaxed);
                return Ok(vec![]);
            }
            // Non-UTF-8 bytes are replaced with U+FFFD — intentional; binary
            // files are already rejected above by the null-byte check.
            let content = String::from_utf8_lossy(&bytes);
            scan_content(&content, &c.rel_path, &regex, config, today)
        })
        .collect();

    let binary_skipped = binary_count.load(Ordering::Relaxed);
    skipped_files += binary_skipped;
    // scanned_files = candidates that passed Phase 1 minus those found binary in Phase 2.
    let scanned_files = candidates.len() - binary_skipped;

    // ----------------------------------------------------------------
    // Phase 3 (serial): Flatten the per-file annotation lists, then
    // sort the combined result by date ascending so the most urgent
    // items appear first.
    // ----------------------------------------------------------------
    let mut annotations: Vec<Annotation> = results?.into_iter().flatten().collect();
    // Unstable sort is faster — NaiveDate is Copy and there is no meaningful
    // tiebreaker for equal dates, so stability adds cost for free.
    annotations.sort_unstable_by_key(|a| a.date);

    Ok(ScanResult {
        annotations,
        scanned_files,
        skipped_files,
    })
}

/// Scan a single file and return all annotations found.
///
/// `abs_path` is used for reading; `rel_path` is stored in the `Annotation` for display.
/// Binary files (detected via null-byte check) return an empty vec.
/// Non-UTF-8 bytes are replaced with U+FFFD — intentional; binary files are
/// already rejected by the null-byte check.
pub fn scan_file(
    abs_path: &Path,
    rel_path: &Path,
    regex: &Regex,
    config: &Config,
    today: NaiveDate,
) -> Result<Vec<Annotation>> {
    let bytes = std::fs::read(abs_path).map_err(|e| Error::Io {
        source: e,
        path: Some(abs_path.to_path_buf()),
    })?;
    if bytes.contains(&0u8) {
        return Ok(vec![]);
    }
    let content = String::from_utf8_lossy(&bytes);
    scan_content(&content, rel_path, regex, config, today)
}

/// Scan a string (file content) for annotations. Exposed separately for unit testing.
pub fn scan_content(
    content: &str,
    rel_path: &Path,
    regex: &Regex,
    config: &Config,
    today: NaiveDate,
) -> Result<Vec<Annotation>> {
    let mut annotations = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        // Fast byte pre-filter: every valid annotation contains '['.
        // Skips the regex engine entirely for the vast majority of lines.
        if !line.contains('[') {
            continue;
        }

        let line_number = line_idx + 1; // 1-based

        for caps in regex.captures_iter(line) {
            let date_str = &caps[2];

            // Parse the date before any heap allocation — on an invalid date
            // the three allocations below are avoided entirely.
            let date = match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => {
                    eprintln!(
                        "warning: invalid date '{}' at {}:{} — skipping",
                        date_str,
                        rel_path.display(),
                        line_number
                    );
                    continue;
                }
            };

            let tag = caps[1].to_uppercase();
            let owner = caps.get(4).map(|m| m.as_str().trim().to_string());
            let message = caps[5].trim().to_string();

            let status = Annotation::compute_status(date, today, config.warn_within_days);

            annotations.push(Annotation {
                file: rel_path.to_path_buf(),
                line: line_number,
                tag,
                date,
                owner,
                message,
                status,
            });
        }
    }

    Ok(annotations)
}

/// Build the annotation-matching regex from the config's tag list.
pub fn build_regex(config: &Config) -> Result<Regex> {
    let pattern = config.annotation_regex_pattern();
    Regex::new(&pattern).map_err(Error::RegexCompile)
}

/// Detect binary files by looking for null bytes in the first 8 KB.
///
/// Retained for unit testing. Not called in the scan pipeline — Phase 2 of
/// `scan()` performs inline binary detection as part of the single `fs::read`
/// call, avoiding a double file open.
#[cfg(test)]
pub(crate) fn is_binary(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;
    let mut buf = [0u8; 8192];
    // BufReader adds overhead for a single fixed-size read; use File directly.
    let n = f.read(&mut buf).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;
    Ok(buf[..n].contains(&0u8))
}

/// Convenience: scan a string with a freshly built regex.
/// Useful for testing and one-off scanning without a filesystem walk.
#[cfg(test)]
pub(crate) fn scan_str(
    content: &str,
    rel_path: &Path,
    config: &Config,
    today: NaiveDate,
) -> Result<Vec<Annotation>> {
    let regex = build_regex(config)?;
    scan_content(content, rel_path, &regex, config, today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::Status;
    use crate::config::Config;
    use std::path::{Path, PathBuf};

    fn today() -> NaiveDate {
        NaiveDate::parse_from_str("2025-06-01", "%Y-%m-%d").unwrap()
    }

    fn default_config() -> Config {
        Config::default()
    }

    // -----------------------------------------------------------------------
    // scan_str helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_finds_expired_todo() {
        let src = "// TODO[2020-01-01]: remove this old code\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "TODO");
        assert_eq!(anns[0].status, Status::Expired);
        assert_eq!(anns[0].line, 1);
        assert_eq!(anns[0].message, "remove this old code");
    }

    #[test]
    fn test_scan_finds_future_fixme() {
        let src = "# FIXME[2099-01-01]: will still be relevant\n";
        let anns = scan_str(src, Path::new("foo.py"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "FIXME");
        assert_eq!(anns[0].status, Status::Ok);
    }

    #[test]
    fn test_scan_ignores_plain_todo() {
        // Plain TODO without brackets must be ignored
        let src = "// TODO: fix this someday\n// FIXME: also this\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert!(anns.is_empty(), "plain TODOs must not be matched");
    }

    #[test]
    fn test_scan_case_insensitive_tag() {
        let src = "// todo[2020-01-01]: lowercase tag should match\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "TODO"); // normalised to upper
    }

    #[test]
    fn test_scan_with_owner() {
        let src = "// TODO[2020-01-01][alice]: remove after migration\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].owner, Some("alice".to_string()));
        assert_eq!(anns[0].message, "remove after migration");
    }

    #[test]
    fn test_scan_without_owner() {
        let src = "// TODO[2020-01-01]: no owner here\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert_eq!(anns[0].owner, None);
    }

    #[test]
    fn test_scan_expiring_soon() {
        // 2025-06-10 is 9 days from today (2025-06-01), within the 14-day window
        let src = "// TODO[2025-06-10]: expiring soon\n";
        let cfg = Config {
            warn_within_days: 14,
            ..Config::default()
        };
        let anns = scan_str(src, Path::new("foo.rs"), &cfg, today()).unwrap();
        assert_eq!(anns[0].status, Status::ExpiringSoon);
    }

    #[test]
    fn test_scan_multiple_annotations() {
        let src = "\
line 1
// TODO[2020-01-01]: expired item
line 3
# FIXME[2099-12-31]: future item
// HACK[2025-06-08]: expiring soon
line 6
";
        let cfg = Config {
            warn_within_days: 14,
            ..Config::default()
        };
        let anns = scan_str(src, Path::new("multi.rs"), &cfg, today()).unwrap();
        assert_eq!(anns.len(), 3);
        // Find each by tag
        let expired = anns.iter().find(|a| a.tag == "TODO").unwrap();
        assert_eq!(expired.status, Status::Expired);
        assert_eq!(expired.line, 2);

        let future = anns.iter().find(|a| a.tag == "FIXME").unwrap();
        assert_eq!(future.status, Status::Ok);
        assert_eq!(future.line, 4);

        let soon = anns.iter().find(|a| a.tag == "HACK").unwrap();
        assert_eq!(soon.status, Status::ExpiringSoon);
        assert_eq!(soon.line, 5);
    }

    #[test]
    fn test_scan_invalid_date_skipped_with_warning() {
        // Invalid date — should not produce an annotation (warning printed to stderr)
        let src = "// TODO[2026-13-45]: invalid date month\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert!(anns.is_empty());
    }

    #[test]
    fn test_scan_sql_comment() {
        let src = "-- TODO[2020-01-01]: drop this column\n";
        let anns = scan_str(src, Path::new("schema.sql"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].message, "drop this column");
    }

    #[test]
    fn test_scan_hash_comment() {
        let src = "# REMOVEME[2020-01-01]: remove this block\n";
        let anns = scan_str(src, Path::new("script.py"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "REMOVEME");
    }

    #[test]
    fn test_scan_temp_tag() {
        let src = "// TEMP[2020-01-01]: temporary workaround\n";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "TEMP");
    }

    #[test]
    fn test_scan_custom_tags_only() {
        let src = "\
// TODO[2020-01-01]: this should not match
// CUSTOM[2020-01-01]: this should match
";
        let cfg = Config {
            tags: vec!["CUSTOM".to_string()],
            ..Config::default()
        };
        // Rebuild is implicit via scan_str which calls build_regex
        let anns = scan_str(src, Path::new("foo.rs"), &cfg, today()).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].tag, "CUSTOM");
    }

    #[test]
    fn test_scan_empty_file() {
        let anns = scan_str("", Path::new("empty.rs"), &default_config(), today()).unwrap();
        assert!(anns.is_empty());
    }

    #[test]
    fn test_scan_annotation_exactly_at_zero_days_remaining() {
        // Same day as today, no warn window → ExpiringSoon (0 <= 0)
        let src = "// TODO[2025-06-01]: due today\n";
        let cfg = Config {
            warn_within_days: 0,
            ..Config::default()
        };
        let anns = scan_str(src, Path::new("foo.rs"), &cfg, today()).unwrap();
        assert_eq!(anns[0].status, Status::ExpiringSoon);
    }

    // -----------------------------------------------------------------------
    // is_binary
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_binary_text_file() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "// TODO[2020-01-01]: normal text file").unwrap();
        assert!(!is_binary(f.path()).unwrap());
    }

    #[test]
    fn test_is_binary_binary_file() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&[0x50, 0x4b, 0x00, 0x04, 0xFF, 0xFE]).unwrap(); // contains null
        assert!(is_binary(f.path()).unwrap());
    }

    // -----------------------------------------------------------------------
    // ScanResult helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_result_categorisation() {
        let today_date = today();
        let expired = Annotation {
            file: PathBuf::from("a.rs"),
            line: 1,
            tag: "TODO".to_string(),
            date: NaiveDate::parse_from_str("2020-01-01", "%Y-%m-%d").unwrap(),
            owner: None,
            message: "expired".to_string(),
            status: Status::Expired,
        };
        let soon = Annotation {
            file: PathBuf::from("b.rs"),
            line: 2,
            tag: "FIXME".to_string(),
            date: NaiveDate::parse_from_str("2025-06-08", "%Y-%m-%d").unwrap(),
            owner: None,
            message: "soon".to_string(),
            status: Status::ExpiringSoon,
        };
        let ok = Annotation {
            file: PathBuf::from("c.rs"),
            line: 3,
            tag: "HACK".to_string(),
            date: NaiveDate::parse_from_str("2099-01-01", "%Y-%m-%d").unwrap(),
            owner: None,
            message: "fine".to_string(),
            status: Status::Ok,
        };
        let _ = today_date; // used in test context, suppress warning
        let result = ScanResult {
            annotations: vec![expired, soon, ok],
            scanned_files: 3,
            skipped_files: 0,
        };
        assert_eq!(result.expired().len(), 1);
        assert_eq!(result.expiring_soon().len(), 1);
        assert_eq!(result.ok().len(), 1);
        assert!(result.has_expired());
        assert!(result.has_expiring_soon());
        assert_eq!(result.total(), 3);
    }

    // -----------------------------------------------------------------------
    // Full filesystem scan (integration-style)
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_directory() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        let mut f1 = std::fs::File::create(dir.path().join("main.rs")).unwrap();
        writeln!(f1, "// TODO[2020-01-01]: expired").unwrap();
        writeln!(f1, "// FIXME[2099-01-01]: future").unwrap();

        let result = scan(dir.path(), &default_config(), today()).unwrap();
        assert_eq!(result.scanned_files, 1);
        assert_eq!(result.annotations.len(), 2);
        assert!(result.has_expired());
    }

    #[test]
    fn test_scan_directory_skips_excluded() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // Create a .git subdirectory with a Rust-ish file
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let mut f = std::fs::File::create(dir.path().join(".git/hooks.rs")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: should be excluded").unwrap();

        // And a normal file
        let mut f2 = std::fs::File::create(dir.path().join("lib.rs")).unwrap();
        writeln!(f2, "// FIXME[2099-01-01]: ok").unwrap();

        let result = scan(dir.path(), &default_config(), today()).unwrap();
        // Only lib.rs should be scanned; the .git file should be excluded
        assert_eq!(result.scanned_files, 1);
        let tags: Vec<_> = result.annotations.iter().map(|a| a.tag.as_str()).collect();
        assert!(!tags.contains(&"TODO"));
        assert!(tags.contains(&"FIXME"));
    }

    #[test]
    fn test_scan_directory_respects_extensions() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // .xyz extension — not in default list
        let mut f = std::fs::File::create(dir.path().join("data.xyz")).unwrap();
        writeln!(f, "// TODO[2020-01-01]: should be skipped").unwrap();

        let result = scan(dir.path(), &default_config(), today()).unwrap();
        assert_eq!(result.scanned_files, 0);
        assert!(result.annotations.is_empty());
    }

    #[test]
    fn test_scan_sorted_by_date_ascending() {
        let src = "\
// TODO[2099-12-31]: far future
// FIXME[2020-01-01]: expired
// HACK[2050-06-15]: mid future
";
        let anns = scan_str(src, Path::new("foo.rs"), &default_config(), today()).unwrap();
        // scan_str doesn't sort; the full scan() call does. Test scan() sorting via a temp dir.
        // (scan_str is not sorted — only scan() sorts)
        // Verify that each item appears in the right order from the lines themselves:
        assert_eq!(anns[0].tag, "TODO");
        assert_eq!(anns[1].tag, "FIXME");
        assert_eq!(anns[2].tag, "HACK");
    }

    #[test]
    fn test_scan_directory_sorted() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("sort.rs")).unwrap();
        writeln!(f, "// TODO[2099-12-31]: far future").unwrap();
        writeln!(f, "// FIXME[2020-01-01]: expired").unwrap();
        writeln!(f, "// HACK[2050-06-15]: mid future").unwrap();

        let result = scan(dir.path(), &default_config(), today()).unwrap();
        let dates: Vec<_> = result.annotations.iter().map(|a| a.date).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        assert_eq!(
            dates, sorted,
            "scan results should be sorted by date ascending"
        );
    }
}
