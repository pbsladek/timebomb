use crate::error::{Error, Result};
use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Returns changed line ranges per relative file path.
///
/// Runs both `git diff --unified=0 <base>` (unstaged) and
/// `git diff --unified=0 --cached <base>` (staged) and merges the results.
/// `validate_git_ref` is called before any subprocess is spawned.
pub fn git_changed_line_ranges(
    repo_root: &Path,
    base: &str,
) -> Result<HashMap<PathBuf, Vec<RangeInclusive<usize>>>> {
    crate::git::validate_git_ref(base)?;

    let unstaged = run_git_diff_lines(repo_root, base, false)?;
    let staged = run_git_diff_lines(repo_root, base, true)?;

    // Merge: extend vecs for the same file key
    let mut merged: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = unstaged;
    for (path, ranges) in staged {
        merged.entry(path).or_default().extend(ranges);
    }

    Ok(merged)
}

/// Run `git diff --unified=0 [--cached] <base>` and parse the output into
/// line ranges per file.
fn run_git_diff_lines(
    repo_root: &Path,
    base: &str,
    cached: bool,
) -> Result<HashMap<PathBuf, Vec<RangeInclusive<usize>>>> {
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--unified=0");
    if cached {
        cmd.arg("--cached");
    }
    cmd.arg(base);
    cmd.current_dir(repo_root);

    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::InvalidArgument("'git' command not found — is git installed?".to_string())
        } else {
            Error::InvalidArgument(format!("failed to spawn git: {}", e))
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.to_lowercase().contains("not a git repository") {
            return Err(Error::InvalidArgument(
                "git diff failed: not a git repository".to_string(),
            ));
        }
        return Err(Error::InvalidArgument(format!(
            "invalid git ref '{}': {}",
            base, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_unified_diff(&stdout))
}

/// Pure function — parse unified diff text into line ranges per file.
///
/// Lines starting with `+++ b/` give the current file (stripping the `b/` prefix).
/// Hunk headers match `^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@`:
///   - capture 1 = new_start (usize)
///   - capture 2 = new_count (usize, absent means 1)
///   - if new_count == 0: pure deletion — no added lines, skip
///   - range = `new_start..=(new_start + new_count - 1)`
///
/// `/dev/null` as the `+++` target means file deleted — skip.
fn hunk_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
            .expect("hardcoded regex is valid")
    })
}

pub fn parse_unified_diff(output: &str) -> HashMap<PathBuf, Vec<RangeInclusive<usize>>> {
    let hunk_re = hunk_re();

    let mut result: HashMap<PathBuf, Vec<RangeInclusive<usize>>> = HashMap::new();
    let mut current_file: Option<PathBuf> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // "/dev/null" means the file was deleted — no added lines to track
            if rest == "/dev/null" {
                current_file = None;
            } else if let Some(path_str) = rest.strip_prefix("b/") {
                current_file = Some(PathBuf::from(path_str));
            } else {
                // Unexpected format — skip this file
                current_file = None;
            }
            continue;
        }

        if line.starts_with("@@") {
            let Some(ref file) = current_file else {
                continue;
            };
            let Some(caps) = hunk_re.captures(line) else {
                continue;
            };

            let new_start: usize = caps[1].parse().unwrap_or(0);
            // Missing count means exactly one line
            let new_count: usize = caps
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);

            // Pure deletion: new_count == 0 — no added lines
            if new_count == 0 {
                continue;
            }

            let range = new_start..=(new_start + new_count - 1);
            result.entry(file.clone()).or_default().push(range);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_unified_diff unit tests ─────────────────────────────────────────

    #[test]
    fn test_parse_unified_diff_basic() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +2,4 @@
+line a
+line b
+line c
+line d
";
        let map = parse_unified_diff(diff);
        let ranges = map
            .get(Path::new("src/main.rs"))
            .expect("file should be present");
        assert_eq!(ranges.len(), 1);
        assert_eq!(*ranges[0].start(), 2);
        assert_eq!(*ranges[0].end(), 5);
    }

    #[test]
    fn test_parse_unified_diff_single_line_no_count() {
        // @@ -1 +5 @@ — no comma → count defaults to 1
        let diff = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1 +5 @@
+added line
";
        let map = parse_unified_diff(diff);
        let ranges = map
            .get(Path::new("foo.rs"))
            .expect("file should be present");
        assert_eq!(ranges.len(), 1);
        assert_eq!(*ranges[0].start(), 5);
        assert_eq!(*ranges[0].end(), 5);
    }

    #[test]
    fn test_parse_unified_diff_pure_deletion() {
        // @@ -3,2 +3,0 @@ — count is 0, so no range should be added
        let diff = "\
--- a/bar.rs
+++ b/bar.rs
@@ -3,2 +3,0 @@
-deleted line 1
-deleted line 2
";
        let map = parse_unified_diff(diff);
        // Either the key is absent or has an empty vec — either way, no ranges
        let ranges = map.get(Path::new("bar.rs"));
        let is_empty = ranges.map(|v| v.is_empty()).unwrap_or(true);
        assert!(is_empty, "pure deletion should produce no line ranges");
    }

    #[test]
    fn test_parse_unified_diff_multiple_files() {
        let diff = "\
--- a/alpha.rs
+++ b/alpha.rs
@@ -1,1 +1,2 @@
+added in alpha 1
+added in alpha 2
--- a/beta.rs
+++ b/beta.rs
@@ -5,1 +5,1 @@
-old line
+new line
";
        let map = parse_unified_diff(diff);
        assert!(
            map.contains_key(Path::new("alpha.rs")),
            "alpha.rs should be present"
        );
        assert!(
            map.contains_key(Path::new("beta.rs")),
            "beta.rs should be present"
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_unified_diff_dev_null_skipped() {
        // +++ /dev/null means the file was deleted — should produce no entry
        let diff = "\
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-line 1
-line 2
-line 3
";
        let map = parse_unified_diff(diff);
        assert!(
            !map.contains_key(Path::new("gone.rs")),
            "/dev/null target should not produce an entry"
        );
        assert!(map.is_empty(), "map should be empty for deleted-file diff");
    }

    #[test]
    fn test_parse_unified_diff_multiple_hunks_same_file() {
        let diff = "\
--- a/multi.rs
+++ b/multi.rs
@@ -1,1 +1,2 @@
+hunk1 line1
+hunk1 line2
@@ -10,1 +11,3 @@
+hunk2 line1
+hunk2 line2
+hunk2 line3
";
        let map = parse_unified_diff(diff);
        let ranges = map
            .get(Path::new("multi.rs"))
            .expect("multi.rs should be present");
        assert_eq!(ranges.len(), 2, "should have two separate hunk ranges");
        // First hunk: +1,2 → 1..=2
        assert_eq!(*ranges[0].start(), 1);
        assert_eq!(*ranges[0].end(), 2);
        // Second hunk: +11,3 → 11..=13
        assert_eq!(*ranges[1].start(), 11);
        assert_eq!(*ranges[1].end(), 13);
    }
}
